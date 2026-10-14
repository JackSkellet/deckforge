// Built by patch-spotify-linux.py against Deckforge's existing dependencies.
// The script supplies the absolute module path at build time.
#[path = "PROTECTED_MANIFEST_SOURCE"]
mod protected_manifest;
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let root = std::path::Path::new(&args[1]);
    let manifest = protected_manifest::read(
        &std::fs::read(root.join("manifest.json"))?,
        std::path::Path::new(&args[2]),
    )?;
    for file in protected_manifest::required_files(&manifest)? {
        file.verify(&std::fs::read(root.join(&file.path))?)?;
    }
    std::fs::write(&args[3], serde_json::to_vec_pretty(&manifest)?)?;
    Ok(())
}
