//! worker_dash.rs — CARD-0243. The FIXED live worker view: no scroll,
//! live values, `\x1b[H` in-place redraw (one initial clear, never
//! again), every frame padded to the SAME height so no residue
//! survives. And the herdr guarantee: while the worker runs, a split
//! pane in the SAME herdr workspace always shows this view — the
//! beekeeper re-checks it each minute and re-opens it if the operator
//! closed it (kill-switch: CADDIS_DASH_NO_ENSURE=1 — TESTS MUST SET
//! IT: an ensure under an inherited HERDR_ENV splits real panes).
//!
//! Read-only forever: the watch loop never spawns work, never writes
//! lineage state (same law as the one-shot board).

/// The frame height every render is padded to. Constant BY LAW: two
/// different lineage states must render the identical line count or
/// the in-place redraw leaves residue (pinned by worker_dash.rs).
const FRAME_HEIGHT: usize = 46;

/// Pad (or, never in practice, truncate) a rendered frame to the fixed
/// height so `\x1b[H` overwrites the whole previous frame.
pub(crate) fn fixed_frame(rendered: &str) -> String {
    let mut lines: Vec<String> = rendered.trim_end().lines().map(str::to_string).collect();
    while lines.len() < FRAME_HEIGHT {
        lines.push(" ".to_string()); // space, not "": a trailing empty
                                     // line would vanish at the final newline and shrink the frame
    }
    lines.truncate(FRAME_HEIGHT);
    lines.join("\n")
}

/// The command a dash pane runs (also the marker searched in pane
/// CONTENT — herdr titles are shell paths and never carry it).
pub(crate) fn dash_command(lineage: &str) -> String {
    format!("caddis worker board --lineage {lineage} --watch")
}

/// The header every dash frame paints — the pane-content marker.
pub(crate) fn dash_marker(lineage: &str) -> String {
    format!("caddis worker board ── lineage {lineage}")
}

/// All pane ids in a `herdr pane list` snapshot.
pub(crate) fn pane_ids(list_json: &str) -> Vec<String> {
    let mut out = Vec::new();
    let needle = "\"pane_id\":\"";
    let mut rest = list_json;
    while let Some(i) = rest.find(needle) {
        let tail = &rest[i + needle.len()..];
        if let Some(end) = tail.find('"') {
            out.push(tail[..end].to_string());
        }
        rest = &rest[i + needle.len()..];
    }
    out
}

/// Run `herdr <args…>` (the estate's herdr is a .cmd shim — Windows
/// needs cmd /c for it).
fn herdr_run(args: &[&str]) -> Option<String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let candidates: &[&str] = &["herdr.exe", "herdr.cmd", "herdr"];
        for name in candidates {
            let cand = dir.join(name);
            if !cand.is_file() {
                continue;
            }
            let use_cmd = name.ends_with(".cmd");
            let mut cmd = if use_cmd {
                let mut c = std::process::Command::new("cmd");
                c.arg("/c").arg(&cand);
                c
            } else {
                std::process::Command::new(&cand)
            };
            let out = cmd.args(args).output().ok()?;
            return Some(String::from_utf8_lossy(&out.stdout).into_owned());
        }
    }
    None
}

/// True when any pane's VISIBLE content is this lineage's dash frame.
fn dash_visible(ws: &str, lineage: &str) -> bool {
    let Some(list) = herdr_run(&["pane", "list", "--workspace", ws]) else {
        return true; // herdr unreachable: never split blindly
    };
    let marker = dash_marker(lineage);
    for pane in pane_ids(&list) {
        // `--lines` returns the BOTTOM rows of the viewport; the frame
        // header sits at the TOP, so read the whole viewport.
        if let Some(view) = herdr_run(&[
            "pane", "read", &pane, "--source", "visible", "--lines", "50",
        ]) {
            if view.contains(&marker) {
                return true;
            }
        }
    }
    false
}

/// Ensure the dash pane exists in THIS workspace (herdr skill laws:
/// sibling split, --no-focus, same cwd). No-op when herdr is absent,
/// when the dash is already visible, or when the kill-switch is set.
pub(crate) fn ensure_herdr_split(lineage: &str, cwd: &str) {
    if std::env::var_os("CADDIS_DASH_NO_ENSURE").is_some() {
        return;
    }
    let Some(ws) = std::env::var_os("HERDR_WORKSPACE_ID") else {
        return; // not inside herdr — nothing to ensure
    };
    let ws = ws.to_string_lossy().into_owned();
    if dash_visible(&ws, lineage) {
        return;
    }
    // Split a sibling pane (never steal focus), then run the dash in it.
    let Some(split_text) = herdr_run(&[
        "pane",
        "split",
        "--current",
        "--direction",
        "right",
        "--cwd",
        cwd,
        "--no-focus",
    ]) else {
        return;
    };
    let Some(pane) = extract_pane_id(&split_text) else {
        return;
    };
    // swallow: fail-safe-by-law — a failed pane run leaves the split empty; the next minute retries
    let _ = herdr_run(&["pane", "run", &pane, &dash_command(lineage)]);
}

/// Pull `"pane_id":"wX:pN"` out of a herdr split response (flat scan;
/// herdr JSON is single-line in practice).
fn extract_pane_id(json: &str) -> Option<String> {
    let needle = "\"pane_id\":\"";
    let i = json.find(needle)? + needle.len();
    let rest = &json[i..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_ids_and_content_marker_parse() {
        let list =
            r#"{"result":{"panes":[{"pane_id":"wC:p1","title":"omp"},{"pane_id":"wC:p9"}]}}"#;
        assert_eq!(pane_ids(list), vec!["wC:p1", "wC:p9"]);
        // The content marker is lineage-unique (the ── is the frame's):
        let view = "● caddis worker board ── lineage watch3 ┄┄┄┄";
        assert!(view.contains(&dash_marker("watch3")));
        assert!(!view.contains(&dash_marker("other")));
        assert_eq!(
            dash_command("watch3"),
            "caddis worker board --lineage watch3 --watch"
        );
    }

    #[test]
    fn fixed_frame_pads_to_constant_height() {
        let short = fixed_frame("one\ntwo");
        let long = fixed_frame(&"x\n".repeat(200));
        assert_eq!(short.lines().count(), FRAME_HEIGHT);
        assert_eq!(long.lines().count(), FRAME_HEIGHT);
    }

    #[test]
    fn split_response_pane_id_extracted() {
        let resp = r#"{"result":{"pane":{"pane_id":"wC:p12","title":"shell"}}}"#;
        assert_eq!(extract_pane_id(resp).as_deref(), Some("wC:p12"));
        assert_eq!(extract_pane_id("no json"), None);
    }
}
