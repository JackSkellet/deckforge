//! Root-scoped local assets for native plugin HTML. No network or vendor-file
//! rewriting. The caller configures WebKit's custom scheme as CORS-enabled so
//! ordinary same-origin module imports/fetches work; responses grant no CORS.
use anyhow::{Context, Result, ensure};
use percent_encoding::percent_decode_str;
use sha2::{Digest, Sha256};
use std::{
    borrow::Cow,
    ffi::CString,
    fs::{File, OpenOptions},
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::OpenOptionsExt},
    },
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use url::Url;
use wry::http::{Method, Request, Response, StatusCode};

pub const SCHEME: &str = "deckforge-plugin";
const LIMIT: u64 = 32 * 1024 * 1024;
#[derive(Clone)]
pub struct Assets {
    root: PathBuf,
    directory: Arc<File>,
    host: String,
    url: Url,
}
impl Assets {
    pub fn new(root: &Path, entry: &Path) -> Result<Self> {
        let root = root
            .canonicalize()
            .context("Cannot resolve plugin asset root")?;
        let entry = if entry.is_absolute() {
            entry.to_owned()
        } else {
            root.join(entry)
        };
        let entry = entry
            .canonicalize()
            .context("Cannot resolve plugin entry")?;
        let relative = entry
            .strip_prefix(&root)
            .context("Plugin entry is outside its asset root")?;
        ensure!(entry.is_file(), "Plugin entry is not a regular file");
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&root)
            .context("Cannot open plugin asset root")?;
        let digest = format!("{:x}", Sha256::digest(root.as_os_str().as_bytes()));
        let host = format!("{}.localhost", &digest[..32]);
        let mut url = Url::parse(&format!("{SCHEME}://{host}/"))?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Invalid asset origin"))?;
            segments.clear();
            for component in relative.components() {
                let Component::Normal(name) = component else {
                    anyhow::bail!("Invalid plugin entry path");
                };
                let name = name.to_str().context("Plugin entry name must be UTF-8")?;
                ensure!(
                    !name.contains(['\\', '\0']) && !name.chars().any(char::is_control),
                    "Invalid plugin entry name"
                );
                segments.push(name);
            }
        }
        Ok(Self {
            root,
            directory: Arc::new(directory),
            host,
            url,
        })
    }
    pub fn url(&self) -> &Url {
        &self.url
    }
    pub fn respond(&self, request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
        let head = request.method() == Method::HEAD;
        if request.method() != Method::GET && !head {
            let mut response = error(StatusCode::METHOD_NOT_ALLOWED, false);
            response
                .headers_mut()
                .insert("Allow", "GET, HEAD".parse().unwrap());
            return response;
        }
        if request.uri().scheme_str() != Some(SCHEME)
            || request.uri().authority().map(|v| v.as_str()) != Some(self.host.as_str())
        {
            return error(StatusCode::FORBIDDEN, head);
        }
        match self.read(request.uri().path(), head) {
            Ok((body, size, content_type)) => response(StatusCode::OK, content_type, size, body),
            Err(status) => error(status, head),
        }
    }
    fn read(
        &self,
        raw: &str,
        head: bool,
    ) -> std::result::Result<(Cow<'static, [u8]>, u64, &'static str), StatusCode> {
        let mut relative = PathBuf::new();
        if !raw.starts_with('/') {
            return Err(StatusCode::BAD_REQUEST);
        }
        // Validate the raw URI before Url's dot-segment normalization. A URL
        // parser alone would erase encoded '..' before the containment check.
        for part in raw[1..].split('/') {
            if part.is_empty() {
                return Err(StatusCode::NOT_FOUND);
            }
            let bytes = part.as_bytes();
            for (index, byte) in bytes.iter().enumerate() {
                if *byte == b'%'
                    && !(bytes.get(index + 1).is_some_and(u8::is_ascii_hexdigit)
                        && bytes.get(index + 2).is_some_and(u8::is_ascii_hexdigit))
                {
                    return Err(StatusCode::BAD_REQUEST);
                }
            }
            let decoded = percent_decode_str(part)
                .decode_utf8()
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            if decoded == "."
                || decoded == ".."
                || decoded.contains(['/', '\\', '\0'])
                || decoded.chars().any(char::is_control)
            {
                return Err(StatusCode::FORBIDDEN);
            }
            relative.push(decoded.as_ref());
        }
        let content_type = mime(&relative);
        let canonical = self
            .root
            .join(relative)
            .canonicalize()
            .map_err(|_| StatusCode::NOT_FOUND)?;
        let relative = canonical
            .strip_prefix(&self.root)
            .map_err(|_| StatusCode::FORBIDDEN)?;
        // Resolve internal symlinks, then pin/open every resolved component with
        // no-follow so subsequent symlink substitutions cannot escape the root.
        let components: Vec<_> = relative.components().collect();
        if components.is_empty() {
            return Err(StatusCode::NOT_FOUND);
        }
        let mut file = self
            .directory
            .try_clone()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        for (index, component) in components.iter().enumerate() {
            let Component::Normal(name) = component else {
                return Err(StatusCode::FORBIDDEN);
            };
            let name = CString::new(name.as_bytes()).map_err(|_| StatusCode::FORBIDDEN)?;
            let flags = libc::O_RDONLY
                | libc::O_CLOEXEC
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | if index + 1 < components.len() {
                    libc::O_DIRECTORY
                } else {
                    0
                };
            // SAFETY: live pinned directory FD, valid C name; each returned FD
            // transfers exactly once to File and closes on every return path.
            let fd = unsafe { libc::openat(file.as_raw_fd(), name.as_ptr(), flags) };
            if fd < 0 {
                return Err(StatusCode::NOT_FOUND);
            }
            file = unsafe { File::from_raw_fd(fd) };
        }
        let metadata = file.metadata().map_err(|_| StatusCode::NOT_FOUND)?;
        if !metadata.is_file() {
            return Err(StatusCode::FORBIDDEN);
        }
        if metadata.len() > LIMIT {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        if head {
            return Ok((Cow::Borrowed(&[]), metadata.len(), content_type));
        }
        let mut bytes = Vec::new();
        file.take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| StatusCode::NOT_FOUND)?;
        if bytes.len() as u64 > LIMIT {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        let size = bytes.len() as u64;
        Ok((Cow::Owned(bytes), size, content_type))
    }
}
fn response(
    status: StatusCode,
    mime: &str,
    size: u64,
    body: Cow<'static, [u8]>,
) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header("Content-Type", mime)
        .header("Content-Length", size.to_string())
        .header("X-Content-Type-Options", "nosniff")
        .header("Cache-Control", "no-store")
        .body(body)
        .unwrap()
}
fn error(status: StatusCode, head: bool) -> Response<Cow<'static, [u8]>> {
    let text = status
        .canonical_reason()
        .unwrap_or("Asset unavailable")
        .as_bytes();
    response(
        status,
        "text/plain; charset=utf-8",
        text.len() as u64,
        if head {
            Cow::Borrowed(&[])
        } else {
            Cow::Borrowed(text)
        },
    )
}
fn mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" | "cjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "aac" => "audio/aac",
        "m4a" => "audio/mp4",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "txt" => "text/plain; charset=utf-8",
        "xml" => "application/xml",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};
    fn fixture() -> (tempfile::TempDir, Assets) {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("ui")).unwrap();
        fs::write(root.path().join("ui/index.html"), "<!doctype html>").unwrap();
        let assets = Assets::new(root.path(), Path::new("ui/index.html")).unwrap();
        (root, assets)
    }
    fn request(assets: &Assets, method: &str, path: &str) -> Response<Cow<'static, [u8]>> {
        assets.respond(
            Request::builder()
                .method(method)
                .uri(format!("{SCHEME}://{}{path}", assets.host))
                .body(vec![])
                .unwrap(),
        )
    }
    #[test]
    fn serves_modules_styles_and_unicode_with_query_and_head() {
        let (root, assets) = fixture();
        fs::write(root.path().join("ui/å #?.mjs"), "export const value=7;").unwrap();
        let reply = request(&assets, "GET", "/ui/%C3%A5%20%23%3F.mjs?cache=123");
        assert_eq!(reply.status(), StatusCode::OK);
        assert_eq!(reply.body().as_ref(), b"export const value=7;");
        assert_eq!(
            reply.headers()["content-type"],
            "text/javascript; charset=utf-8"
        );
        assert_eq!(reply.headers()["x-content-type-options"], "nosniff");
        assert!(!reply.headers().contains_key("access-control-allow-origin"));
        let head = request(&assets, "HEAD", "/ui/%C3%A5%20%23%3F.mjs?other");
        assert_eq!(head.status(), StatusCode::OK);
        assert!(head.body().is_empty());
        assert_eq!(
            head.headers()["content-length"],
            reply.headers()["content-length"]
        );
        fs::write(root.path().join("ui/style.css"), "body{}").unwrap();
        assert_eq!(
            request(&assets, "GET", "/ui/style.css").headers()["content-type"],
            "text/css; charset=utf-8"
        );
    }
    #[test]
    fn origin_is_stable_per_root_and_entry_url_encodes_path() {
        let (root, a) = fixture();
        fs::write(root.path().join("ui/你好 #?.html"), "x").unwrap();
        let b = Assets::new(root.path(), Path::new("ui/你好 #?.html")).unwrap();
        assert_eq!(a.url().host_str(), b.url().host_str());
        assert_eq!(b.url().query(), None);
        assert_eq!(b.url().fragment(), None);
        assert!(b.url().path().contains("%23%3F"));
        assert_eq!(
            b.respond(
                Request::builder()
                    .uri(b.url().as_str())
                    .body(vec![])
                    .unwrap()
            )
            .status(),
            StatusCode::OK
        );
        let (other_root, other) = fixture();
        assert_ne!(a.url().host_str(), other.url().host_str());
        assert!(Assets::new(root.path(), &other_root.path().join("ui/index.html")).is_err());
    }
    #[test]
    fn rejects_methods_wrong_origin_and_encoded_path_attacks() {
        let (_, assets) = fixture();
        let post = request(&assets, "POST", "/ui/index.html");
        assert_eq!(post.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(post.headers()["allow"], "GET, HEAD");
        for path in [
            "/../outside",
            "/%2e%2e/outside",
            "/ui/%2E%2e/file",
            "/ui/a%2fb",
            "/ui/a%5cb",
            "/ui/a%00b",
            "/ui/%ff",
            "/ui/%zz",
        ] {
            assert!(
                !request(&assets, "GET", path).status().is_success(),
                "{path}"
            );
        }
        for url in [
            format!("https://{}/ui/index.html", assets.host),
            format!("{SCHEME}://wrong.localhost/ui/index.html"),
            format!("{SCHEME}://{}:80/ui/index.html", assets.host),
        ] {
            assert_eq!(
                assets
                    .respond(Request::builder().uri(url).body(vec![]).unwrap())
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
    }
    #[test]
    fn internal_symlink_allowed_external_symlink_and_fifo_rejected() {
        let (root, assets) = fixture();
        let external = tempfile::tempdir().unwrap();
        fs::write(external.path().join("private"), "PRIVATE_BYTES").unwrap();
        symlink("ui/index.html", root.path().join("internal.html")).unwrap();
        assert_eq!(
            request(&assets, "GET", "/internal.html").status(),
            StatusCode::OK
        );
        fs::write(root.path().join("module-data"), "export default 1;").unwrap();
        symlink("module-data", root.path().join("module.mjs")).unwrap();
        assert_eq!(
            request(&assets, "GET", "/module.mjs").headers()["content-type"],
            "text/javascript; charset=utf-8"
        );
        symlink(external.path().join("private"), root.path().join("escape")).unwrap();
        let denied = request(&assets, "GET", "/escape");
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        assert!(!String::from_utf8_lossy(denied.body()).contains("PRIVATE"));
        symlink(external.path(), root.path().join("outside")).unwrap();
        assert_eq!(
            request(&assets, "GET", "/outside/private").status(),
            StatusCode::FORBIDDEN
        );
        let name = CString::new(root.path().join("fifo").as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let start = std::time::Instant::now();
        assert_eq!(
            request(&assets, "GET", "/fifo").status(),
            StatusCode::FORBIDDEN
        );
        assert!(start.elapsed() < std::time::Duration::from_millis(100));
    }
    #[test]
    fn size_bound_and_generic_errors_do_not_expose_filesystem() {
        let (root, assets) = fixture();
        File::create(root.path().join("large"))
            .unwrap()
            .set_len(LIMIT + 1)
            .unwrap();
        for method in ["GET", "HEAD"] {
            let r = request(&assets, method, "/large");
            assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
            if method == "HEAD" {
                assert!(r.body().is_empty());
            }
        }
        let missing = request(&assets, "GET", "/absent.js");
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        assert_eq!(missing.body().as_ref(), b"Not Found");
        assert_eq!(mime(Path::new("font.WOFF2")), "font/woff2");
        assert_eq!(mime(Path::new("module.wasm")), "application/wasm");
        assert_eq!(mime(Path::new("clip.mp3")), "audio/mpeg");
    }
}
