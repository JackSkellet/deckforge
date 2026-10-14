//! Close only same-user processes whose actual executable matches the selected file.
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::fs::MetadataExt,
    },
    path::Path,
};

#[cfg(test)]
pub fn matching_pids(executable: &Path, args: &[String]) -> Result<Vec<u32>> {
    let target = executable.canonicalize()?;
    let uid = unsafe { libc::getuid() };
    Ok(fs::read_dir("/proc")?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let pid = entry.file_name().to_str()?.parse::<u32>().ok()?;
            (pid > 1
                && entry.metadata().is_ok_and(|m| m.uid() == uid)
                && fs::read_link(entry.path().join("exe")).is_ok_and(|exe| exe == target)
                && matches_arguments(&entry.path(), args))
            .then_some(pid)
        })
        .collect())
}
#[cfg(test)]
pub fn close(executable: &Path, force: bool) -> Result<usize> {
    close_matching(executable, force, &[])
}
fn matches_arguments(process: &Path, args: &[String]) -> bool {
    if args.is_empty() {
        return true;
    }
    fs::read(process.join("cmdline")).is_ok_and(|bytes| {
        let actual: Vec<_> = bytes.split(|&b| b == 0).skip(1).take(args.len()).collect();
        actual.len() == args.len() && actual.iter().zip(args).all(|(a, b)| *a == b.as_bytes())
    })
}
pub struct PinnedProcess {
    pub pid: u32,
    fd: OwnedFd,
    target: std::sync::Arc<(std::path::PathBuf, Vec<String>)>,
}
impl PinnedProcess {
    pub fn alive(&self) -> bool {
        let mut event = libc::pollfd {
            fd: self.fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        unsafe { libc::poll(&mut event, 1, 0) == 0 }
    }
    pub fn signal(&self, force: bool) -> Result<bool> {
        ensure!(
            self.pid != std::process::id(),
            "Use Deckforge's Quit command to close Deckforge"
        );
        if !self.matches_now() {
            return Ok(false);
        }
        let signal = if force { libc::SIGKILL } else { libc::SIGTERM };
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.fd.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(false);
            }
            return Err(error).context("Request application termination");
        }
        Ok(true)
    }
    pub fn matches_now(&self) -> bool {
        let process = std::path::PathBuf::from(format!("/proc/{}", self.pid));
        self.alive()
            && process
                .metadata()
                .is_ok_and(|m| m.uid() == unsafe { libc::getuid() })
            && fs::read_link(process.join("exe")).is_ok_and(|exe| exe == self.target.0)
            && matches_arguments(&process, &self.target.1)
            && self.alive()
    }
}

pub fn pinned_matches(executable: &Path, args: &[String]) -> Result<Vec<PinnedProcess>> {
    pin_matches(executable, args, false)
}
pub fn pinned_for_activation(executable: &Path, args: &[String]) -> Result<Vec<PinnedProcess>> {
    pin_matches(executable, args, true)
}
fn pin_matches(executable: &Path, args: &[String], allow_self: bool) -> Result<Vec<PinnedProcess>> {
    let target = executable
        .canonicalize()
        .context("Resolve application executable")?;
    ensure!(
        allow_self || target != std::env::current_exe()?.canonicalize()?,
        "Use Deckforge's Quit command to close Deckforge"
    );
    let uid = unsafe { libc::getuid() };
    let identity = std::sync::Arc::new((target.clone(), args.to_vec()));
    let mut matches = Vec::new();
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        if pid <= 1 || (!allow_self && pid == std::process::id()) {
            continue;
        }
        // Pin before inspection, then reject an exited identity even if /proc's
        // numeric PID has been reused. Signals never target a recycled PID.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) } as i32;
        if fd < 0 {
            continue;
        }
        let process = PinnedProcess {
            pid,
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
            target: identity.clone(),
        };
        if entry.metadata().is_ok_and(|m| m.uid() == uid)
            && fs::read_link(entry.path().join("exe")).is_ok_and(|exe| exe == target)
            && matches_arguments(&entry.path(), args)
            && process.alive()
        {
            matches.push(process);
        }
    }
    Ok(matches)
}

#[cfg(test)]
pub fn close_matching(executable: &Path, force: bool, args: &[String]) -> Result<usize> {
    let mut count = 0;
    for process in pinned_matches(executable, args)? {
        count += usize::from(process.signal(force)?);
    }
    ensure!(
        count > 0,
        "No running process matches this Linux executable"
    );
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::Command,
        time::{Duration, Instant},
    };
    #[test]
    fn activation_can_pin_self_but_signals_cannot_target_self() {
        let processes = pinned_for_activation(&std::env::current_exe().unwrap(), &[]).unwrap();
        let process = processes
            .iter()
            .find(|p| p.pid == std::process::id())
            .expect("Own process must be available for activation");
        assert!(process.matches_now());
        assert!(
            process
                .signal(false)
                .unwrap_err()
                .to_string()
                .contains("Quit")
        );
    }
    #[test]
    fn pinned_match_rejects_a_process_that_execs_a_different_image() {
        use std::io::Write;
        let shell = Path::new("/bin/sh").canonicalize().unwrap();
        let script = "read deckforge_close_test_line; exec /usr/bin/sleep 30";
        let marker = format!("deckforge-close-{}", uuid::Uuid::new_v4());
        let args = vec!["-c".into(), script.into(), marker];
        let mut child = Command::new(&shell)
            .args(&args)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let process = loop {
            if let Some(process) = pinned_matches(&shell, &args).unwrap().pop() {
                break process;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        };
        child.stdin.take().unwrap().write_all(b"\n").unwrap();
        while fs::read_link(format!("/proc/{}/exe", child.id())).unwrap() == shell {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(process.alive());
        assert!(!process.matches_now());
        assert!(!process.signal(true).unwrap());
        assert!(child.try_wait().unwrap().is_none());
        child.kill().unwrap();
        child.wait().unwrap();
    }
    #[test]
    fn launcher_arguments_isolate_processes_sharing_one_executable() {
        let target = Path::new("/usr/bin/sleep");
        let selected_arg = format!("300.{}", uuid::Uuid::new_v4().as_u128());
        let other_arg = format!("301.{}", uuid::Uuid::new_v4().as_u128());
        let mut selected = Command::new(target).arg(&selected_arg).spawn().unwrap();
        let mut other = Command::new(target).arg(&other_arg).spawn().unwrap();
        let args = vec![selected_arg];
        // Wait until exec has replaced the forked parent image.
        let deadline = Instant::now() + Duration::from_secs(2);
        while matching_pids(target, &args).unwrap().is_empty() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(matching_pids(target, &args).unwrap(), vec![selected.id()]);
        assert_eq!(close_matching(target, false, &args).unwrap(), 1);
        assert!(!selected.wait().unwrap().success());
        assert!(other.try_wait().unwrap().is_none());
        other.kill().unwrap();
        other.wait().unwrap();
    }
    #[test]
    fn closes_only_the_exact_executable_and_rejects_self() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("owned sleep");
        fs::copy("/usr/bin/sleep", &target).unwrap();
        // A concurrent test fork can briefly inherit the copy's write fd before
        // exec closes it. Retry only this fixture-creation ETXTBSY race.
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut selected = loop {
            match Command::new(&target).arg("30").spawn() {
                Ok(child) => break child,
                Err(error)
                    if error.raw_os_error() == Some(libc::ETXTBSY) && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(error) => panic!("Could not start owned fixture: {error}"),
            }
        };
        let mut other = Command::new("/usr/bin/sleep").arg("30").spawn().unwrap();
        let result = close(&target, false);
        let deadline = Instant::now() + Duration::from_secs(2);
        while selected.try_wait().unwrap().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let ended = selected.try_wait().unwrap().is_some();
        let other_alive = other.try_wait().unwrap().is_none();
        let _ = selected.kill();
        let _ = selected.wait();
        let _ = other.kill();
        let _ = other.wait();
        assert_eq!(result.unwrap(), 1);
        assert!(ended && other_alive);
        assert!(close(&target, false).is_err());
        assert!(close(&std::env::current_exe().unwrap(), false).is_err());
    }
}
