//! build.rs — CARD-0227 + CARD-0328. Embed git short hash at compile
//! time, and rerun when the COMMIT actually moves: HEAD is a
//! `ref: refs/heads/<branch>` POINTER — a same-branch commit rewrites
//! the ref file, not HEAD, so the ref (or packed-refs) must be watched
//! too or the binary keeps the predecessor's hash.

use std::process::Command;

fn main() {
    let hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(o.stdout)
            } else {
                None
            }
        })
        .map(|b| String::from_utf8_lossy(&b).trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=CADDIS_GIT_HASH={hash}");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    watch_head_ref();
}

/// CARD-0328: watch the ref HEAD names — the loose ref when present,
/// packed-refs otherwise (fresh clones pack their refs). A detached
/// HEAD is a raw hash: HEAD itself changed, nothing more to watch.
fn watch_head_ref() {
    let Ok(head) = std::fs::read_to_string("../../.git/HEAD") else {
        return;
    };
    let Some(branch) = head.trim().strip_prefix("ref: ") else {
        return;
    };
    let loose = format!("../../.git/{branch}");
    if std::path::Path::new(&loose).is_file() {
        println!("cargo:rerun-if-changed={loose}");
    } else {
        println!("cargo:rerun-if-changed=../../.git/packed-refs");
    }
}
