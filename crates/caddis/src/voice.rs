//! Register the chair's voice label. Override with CADDIS_VOICE_BIN (tests).

use std::path::Path;
use std::process::Command;

pub fn register(label: &str) -> Result<(), String> {
    if let Some(bin) = std::env::var_os("CADDIS_VOICE_BIN") {
        return spawn(Path::new(&bin), &["register", label]);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .ok_or_else(|| "HOME is unset".to_string())?;
    let script = Path::new(&home)
        .join(".pi")
        .join("agent")
        .join("bin")
        .join("peleida-voice.py");
    if !script.is_file() {
        return Err(format!("voice helper missing: {}", script.display()));
    }
    let py = if cfg!(windows) { "python" } else { "python3" };
    let script_s = script.to_string_lossy();
    spawn(Path::new(py), &[&script_s, "register", label])
}

fn spawn(bin: &Path, args: &[&str]) -> Result<(), String> {
    let py = bin.extension().and_then(|e| e.to_str()) == Some("py");
    let mut cmd = if py {
        Command::new(if cfg!(windows) { "python" } else { "python3" })
    } else {
        Command::new(bin)
    };
    if py {
        cmd.arg(bin);
    }
    let status = cmd
        .args(args)
        .status()
        .map_err(|e| format!("voice spawn: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "voice register exited {}",
            status.code().unwrap_or(-1)
        ))
    }
}
