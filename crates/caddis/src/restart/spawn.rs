//! restart/spawn.rs — CARD-0318. The spawn transaction's seat helpers,
//! moved verbatim from restart.rs to hold the 280-line ceiling (the
//! worker_board_* split precedent). Pure move; no behavior change.

use std::fs;

use crate::receipt;

/// The seat's kind/model, hinted from the arm receipt (plain field
/// read — same lineage-dir trust domain as ready.root).
pub(crate) fn seat_identity(dir: &std::path::Path) -> (String, String) {
    let arm = fs::read_to_string(dir.join("arm.receipt")).unwrap_or_default();
    let kind = receipt::extract_field(arm.as_bytes(), "kind").unwrap_or_default();
    let model = receipt::extract_field(arm.as_bytes(), "model").unwrap_or_default();
    (kind, model)
}

/// The line that boots the successor seat with the pointer as its
/// first prompt (single quotes — CARD-0149: never backticks; the
/// pointer's lineage id carries no quotes).
pub(crate) fn seat_cmd(lineage: &str, kind: &str, model: &str) -> String {
    let p = crate::restart::pointer(lineage);
    match kind {
        "omp" => format!("omp --model {model} '{p}'"),
        "claude" => format!("claude --model {model} '{p}'"),
        "qpi" => format!("pi --model {model} '{p}'"),
        _ => p,
    }
}

/// Pull a `wX:tY`/`wX:pN`-shaped pane id out of split output JSON.
pub(crate) fn extract_pane_id(text: &str) -> Option<String> {
    let needle = "\"pane_id\":\"";
    let mut rest = text;
    while let Some(i) = rest.find(needle) {
        let tail = &rest[i + needle.len()..];
        if let Some(end) = tail.find('"') {
            let id = tail[..end].to_string();
            if id.contains(':') {
                return Some(id);
            }
        }
        rest = tail;
    }
    None
}
