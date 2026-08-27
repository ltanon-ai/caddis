//! PATH lookup. Windows honours PATHEXT so a test stub `caddis-warden.cmd` counts.

use std::env;
use std::path::Path;

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
