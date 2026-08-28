//! collector.rs — P1 slice 1: seed the seat registry from the desktop
//! OMP-style provider catalog (`~/.pi/agent/models.json`, 13 providers,
//! verified live 2026-08-26 — ORGANS-PLANNING tick log).
//!
//! The collector is the ONE writer allowed to create the initial stream
//! (plan P1: "Seed input: desktop models.json → collector writes the
//! initial card stream"). After the seed, every change rides the
//! warden-gated propose→operator-confirm path (P1 slice 3) — the router
//! council ruling (collectors propose, operators rule) transposed: the
//! SEED is facts-only, no taste.
//!
//! Derivation laws (deterministic, zero taste — anything judgment-shaped
//! stays an operator ruling):
//! - **provider card per provider row**; `id` = the models.json key;
//!   `base_url` = the row's `baseUrl` or "" (honest blank — openai-codex
//!   carries its transport per-model); `lane_type` = `http` when the
//!   provider row OR any of its models names an `api`/`baseUrl`, else the
//!   provider is SKIPPED WITH CAUSE (fail-closed: the collector never
//!   guesses bridge/cli — those are ruled, not derived).
//! - **seat card per model row**; `id` = `<provider>/<model id>`; family
//!   = provider id; `caps` = 1 (serialized-by-default, F4); `state` =
//!   `probing` with `since_epoch_s` = 0 (never probed: honest — a fresh
//!   registry selects NOTHING until probes flip seats live, F10). The
//!   PROVIDER card's `caps` comes from the Ruling-7 table
//!   ([`crate::caps::ruled_caps`] — ollama/ollama-cloud = 1, ceiling 2),
//!   a transcribed ruling, not taste.
//! - **cost_class from measured cost**: input+output both 0 → `free`;
//!   any positive → `mid`. `premium` is NEVER collector-derived — it is
//!   taste, set only by a ruling through the edit path.
//! - **NO SECRETS, EVER**: the source `apiKey` is copied ONLY when it is
//!   a file PATH (`auth_path`, Ruling 9 vault-path law). A raw inline key
//!   produces `auth_path: ""` — the credential stays in models.json and
//!   never enters the stream (which is world-readable by design). The
//!   tests pin this by scanning the rendered stream for the raw key.
//! - **Provenance without clocks**: `source` = `models.json#<digest8>` —
//!   deterministic, so the same catalog yields byte-identical cards and
//!   the seed REGENERATES IDEMPOTENTLY (Done-When). No timestamps.
//! - Model rows missing `id`/`contextWindow`/`maxTokens`, or carrying
//!   non-finite/negative numbers: SKIPPED WITH CAUSE, never guessed.

use crate::registry::{Card, ProviderCard, SeatCard};
use crate::sha256;

/// What the collector produced: the deterministic seed cards (providers
/// first, then seats, each sorted by id — stable across runs) and every
/// skip with its cause (honesty surface: a skipped provider/model is
/// REPORTED, never silently dropped).
#[derive(Debug, Clone, PartialEq)]
pub struct CollectReport {
    pub cards: Vec<Card>,
    pub skipped: Vec<String>,
}

/// Path-like credential test: the apiKey VALUE is only ever carried when
/// it names a FILE. Windows and POSIX path shapes; anything else (a raw
/// bearer token) is credential material the stream must not carry.
fn is_pathlike(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() || t.contains('\n') {
        return false;
    }
    // Absolute paths (C:\..., /..., \\server\...), explicit file: URIs,
    // or a relative path with a separator. A bare word with no separator
    // is treated as a token, not a path — fail toward NOT copying.
    t.starts_with("file:")
        || t.starts_with('/')
        || t.starts_with('\\')
        || (t.len() >= 2 && t.as_bytes()[1] == b':' && t.as_bytes()[2] == b'\\')
        || t.contains('/')
        || t.contains('\\')
}

/// The deterministic provenance label for a catalog's bytes: THE sha256 of
/// the catalog (single hash — fixed 2026-08-28 with stream_digest; passing
/// an already-computed digest into `sha256::hex` double-hashed it).
fn source_label(text: &str) -> String {
    format!("models.json#{}", &sha256::hex(text.as_bytes())[..8])
}

/// Parse the catalog and derive the seed cards. `text` is the raw
/// models.json content. Malformed JSON is an ERROR (the collector never
/// seeds from a catalog it cannot fully read); individual rows skip with
/// cause.
pub fn collect(text: &str) -> Result<CollectReport, String> {
    let v = crate::json::parse(text).map_err(|e| format!("models.json: {}", e.msg))?;
    let providers = v
        .get("providers")
        .and_then(|p| p.as_obj())
        .ok_or("models.json: missing \"providers\" object")?;

    let source = source_label(text);
    let mut report = CollectReport {
        cards: Vec::new(),
        skipped: Vec::new(),
    };
    let mut provider_cards: Vec<ProviderCard> = Vec::new();
    let mut seat_cards: Vec<SeatCard> = Vec::new();

    for (pid, prow) in providers {
        let prow = match prow.as_obj() {
            Some(o) => o,
            None => {
                report
                    .skipped
                    .push(format!("provider {pid}: row is not an object"));
                continue;
            }
        };
        // Gather the model rows (absent = empty provider: card, no seats).
        let empty: Vec<crate::json::Value> = Vec::new();
        let models: &[crate::json::Value] = prow
            .iter()
            .find(|(k, _)| k == "models")
            .and_then(|(_, mv)| mv.as_arr())
            .unwrap_or(&empty);

        let base_url = prow
            .iter()
            .find(|(k, _)| k == "baseUrl")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("")
            .to_string();
        let api_word = prow
            .iter()
            .find(|(k, _)| k == "api")
            .and_then(|(_, v)| v.as_str())
            .map(|s| s.to_string());
        let model_api = models
            .iter()
            .find_map(|m| m.get("api").and_then(|a| a.as_str()).map(|s| s.to_string()));
        let lane_type = if api_word.is_some() || model_api.is_some() || !base_url.is_empty() {
            crate::LaneType::Http
        } else {
            report.skipped.push(format!(
                "provider {pid}: no api/baseUrl at provider or model level — lane type is a RULING, not derived (skipped)"
            ));
            continue;
        };

        // Auth: vault PATH only. Raw key => honest blank, value never copied.
        let raw_key = prow
            .iter()
            .find(|(k, _)| k == "apiKey")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("");
        let auth_path = if is_pathlike(raw_key) {
            raw_key.trim().to_string()
        } else {
            String::new()
        };

        provider_cards.push(ProviderCard {
            id: pid.clone(),
            lane_type,
            base_url,
            auth_path,
            // Ruling 7 table (ollama/ollama-cloud = 1), else the F4
            // serialized default — transcribed fact, not taste.
            caps: crate::caps::ruled_caps(pid),
            source: source.clone(),
        });

        for m in models {
            let mid = match m.get("id").and_then(|i| i.as_str()) {
                Some(s) if !s.is_empty() => s,
                _ => {
                    report
                        .skipped
                        .push(format!("model under {pid}: missing/empty \"id\""));
                    continue;
                }
            };
            let flat_num = |key: &str| -> Option<f64> {
                m.get(key)
                    .and_then(|c| c.as_f64())
                    .filter(|n| n.is_finite() && *n >= 0.0)
            };
            // cost is a nested object {input, output} (absent = 0: the
            // catalog's subscription/local lanes bill nothing).
            let cost_num = |key: &str| -> f64 {
                m.get("cost")
                    .and_then(|c| c.get(key))
                    .and_then(|v| v.as_f64())
                    .filter(|n| n.is_finite() && *n >= 0.0)
                    .unwrap_or(0.0)
            };
            let (Some(ctx), Some(max_tok)) = (flat_num("contextWindow"), flat_num("maxTokens"))
            else {
                report.skipped.push(format!(
                    "model {pid}/{mid}: missing/invalid contextWindow|maxTokens"
                ));
                continue;
            };
            let cost_in = cost_num("input");
            let cost_out = cost_num("output");
            let cost_class = if cost_in == 0.0 && cost_out == 0.0 {
                crate::CostClass::Free
            } else {
                crate::CostClass::Mid
            };
            seat_cards.push(SeatCard {
                id: format!("{pid}/{mid}"),
                provider: pid.clone(),
                family: pid.clone(),
                model: mid.to_string(),
                lane_type,
                cost_class,
                state: crate::SeatState::Probing,
                // 0 = no clock data: the deterministic seed. The TTL
                // machine reads it as "never probed" (first probe due
                // now) — never "probed at epoch".
                since_epoch_s: 0,
                caps: 1,
                cost_in_usd_per_mtok: cost_in,
                cost_out_usd_per_mtok: cost_out,
                context_window: ctx as u64,
                max_tokens: max_tok as u64,
                source: source.clone(),
            });
        }
    }

    // Deterministic order: providers by id, then seats by id (BTree sort).
    provider_cards.sort_by(|a, b| a.id.cmp(&b.id));
    seat_cards.sort_by(|a, b| a.id.cmp(&b.id));
    report
        .cards
        .extend(provider_cards.into_iter().map(Card::Provider));
    report.cards.extend(seat_cards.into_iter().map(Card::Seat));
    Ok(report)
}

/// Render the seed STREAM bytes from a catalog: deterministic, LF lines,
/// exactly what [`crate::registry::render_seed`] produces for the
/// collected cards. Re-running on the same catalog yields the same bytes
/// (the idempotency Done-When).
pub fn render_seed_from(text: &str) -> Result<(String, CollectReport), String> {
    let report = collect(text)?;
    let bytes = crate::registry::render_seed(&report.cards);
    Ok((bytes, report))
}

// ---------------------------------------------------------------------------
// P4 slice 1: the one-time organ home bootstrap
// ---------------------------------------------------------------------------

use std::fs;
use std::path::Path;

/// What a [`seed_once`] call did. The stream is created EXACTLY ONCE per
/// home; after that the append-only edit path owns every change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedOutcome {
    /// The home held no stream; the seed wrote it and proved the view.
    Created {
        rows: usize,
        skipped: usize,
        view_synced: bool,
    },
    /// The stream already exists and a fresh render of the SAME catalog is
    /// byte-identical (the idempotency Done-When). Nothing was rewritten.
    AlreadySeeded { rows: usize, view_synced: bool },
}

/// Seed the organ home ONCE from the desktop catalog (P4 slice 1).
///
/// Law: the collector is the ONE writer allowed to CREATE the initial
/// stream; a home that already holds a stream is NEVER re-seeded. When a
/// fresh render of `models_text` is byte-identical to the existing stream
/// the call is an idempotent no-op (view still proven); when it differs
/// the call REFUSES — from the seed moment the stream is the truth, and
/// every change rides the warden-gated propose→confirm path (P1 slice 3),
/// never a clobber.
pub fn seed_once(
    models_text: &str,
    stream_path: &Path,
    view_path: &Path,
) -> Result<SeedOutcome, String> {
    let (bytes, report) = render_seed_from(models_text)?;
    let count_rows = |text: &str| text.lines().filter(|l| !l.trim().is_empty()).count();
    match fs::read_to_string(stream_path) {
        Ok(have) => {
            if have != bytes {
                return Err(format!(
                    "refused: {} already holds a seeded stream ({} rows) that differs \
                     from this catalog's render; the stream is the truth — changes ride \
                     the warden-gated edit path, never a re-seed",
                    stream_path.display(),
                    count_rows(&have)
                ));
            }
            let rows = count_rows(&have);
            let (_, view_synced) = crate::registry::load_and_sync(stream_path, view_path)
                .map_err(|e| format!("view sync after seed: {e}"))?;
            Ok(SeedOutcome::AlreadySeeded { rows, view_synced })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(dir) = stream_path.parent() {
                fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
            }
            // Creation is the ONE non-append write the stream ever gets:
            // tmp sibling + rename, so a crash never leaves a half stream.
            let mut tmp = stream_path.to_path_buf();
            let mut name = stream_path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "seats.jsonl".into());
            name.push_str(".tmp");
            tmp.set_file_name(name);
            fs::write(&tmp, &bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
            fs::rename(&tmp, stream_path)
                .map_err(|e| format!("rename into {}: {e}", stream_path.display()))?;
            let (_, view_synced) = crate::registry::load_and_sync(stream_path, view_path)
                .map_err(|e| format!("view sync after seed: {e}"))?;
            Ok(SeedOutcome::Created {
                rows: report.cards.len(),
                skipped: report.skipped.len(),
                view_synced,
            })
        }
        Err(e) => Err(format!("read {}: {e}", stream_path.display())),
    }
}

#[cfg(test)]
#[path = "collector_tests.rs"]
mod tests;
