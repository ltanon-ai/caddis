//! session.rs — CARD-0125/0126: omp verify writes session.receipt.
//!
//! OMP-only. Fixture via CADDIS_WARDEN_RECEIPT. Tests set
//! CADDIS_SKIP_WARDEN=1 so cargo test never spawns a live warden.

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::receipt;

/// After a clean omp drain, persist a session receipt next to ARM.
pub fn write_on_omp_verify(dir: &Path, kind: &str, model: &str) -> Result<(), String> {
    if kind != "omp" {
        return Ok(());
    }
    fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let ts = receipt::timestamp();
    let text = format!(
        "kind=omp\nmodel={model}\nevent=rotate-verify\nts={ts}\n---\n{}",
        warden_body()
    );
    let path = dir.join("session.receipt");
    fs::write(&path, text.as_bytes()).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

#[allow(clippy::single_match)] // if-let-Ok is a gate swallow
fn warden_body() -> String {
    match env::var("CADDIS_WARDEN_RECEIPT") {
        Ok(path) => {
            return fs::read_to_string(&path)
                .unwrap_or_else(|e| format!("warden=unreadable {e}\n"));
        }
        Err(_) => {}
    }
    if env::var("CADDIS_SKIP_WARDEN").ok().as_deref() == Some("1") {
        return "warden=offline\n".into();
    }
    spawn_warden_receipt()
}

fn spawn_warden_receipt() -> String {
    let bin = env::var("CADDIS_WARDEN_BIN").unwrap_or_else(|_| "caddis-warden".into());
    match Command::new(&bin)
        .args(["receipt", "--from", "omp", "--since", "24"])
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => "warden=offline\n".into(),
    }
}
