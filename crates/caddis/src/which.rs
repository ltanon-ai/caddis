//! PATH lookup. Windows honours PATHEXT so a test stub `caddis-warden.cmd` counts.

use std::env;
use std::path::Path;
use std::process::Command;

pub fn warden_on_path() -> bool {
    command_on_path("caddis-warden")
}

pub fn command_on_path(name: &str) -> bool {
    let path = match env::var_os("PATH") {
        Some(p) => p,
        None => return false,
    };
    env::split_paths(&path).any(|dir| candidate_exists(&dir, name))
}

fn candidate_exists(dir: &Path, name: &str) -> bool {
    if dir.join(name).is_file() {
        return true;
    }
    #[cfg(windows)]
    {
        let pathext = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        for ext in pathext.split(';').filter(|e| !e.is_empty()) {
            let file = if ext.starts_with('.') {
                format!("{name}{ext}")
            } else {
                format!("{name}.{ext}")
            };
            if dir.join(&file).is_file() {
                return true;
            }
        }
    }
    false
}

/// Run `herdr <args…>` (CARD-0309/0310). The estate's herdr is a .cmd
/// shim (E2): cmd /c for those, plain for .exe. CADDIS_HERDR_BIN
/// overrides for hermetic tests. A nonzero exit is a failed call (None)
/// — bare options once exited 2 with empty stdout and read as a
/// successful empty split.
pub fn herdr(args: &[&str]) -> Option<String> {
    if let Some(bin) = env::var_os("CADDIS_HERDR_BIN") {
        return run(Command::new(bin).args(args));
    }
    let path = env::var_os("PATH").unwrap_or_default();
    for dir in env::split_paths(&path) {
        for name in ["herdr.exe", "herdr.cmd", "herdr"] {
            let cand = dir.join(name);
            if !cand.is_file() {
                continue;
            }
            if name.ends_with(".cmd") {
                return run(Command::new("cmd").arg("/c").arg(&cand).args(args));
            }
            return run(Command::new(&cand).args(args));
        }
    }
    None
}

fn run(cmd: &mut Command) -> Option<String> {
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}
