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
//!   = provider id; `caps` = 1 (serialized-by-default, F4 — Ruling 7
//!   per-provider caps land in slice 2); `state` = `probing` (never
//!   probed: honest — a fresh registry selects NOTHING until probes flip
//!   seats live, F10).
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

/// The deterministic provenance label for a catalog's bytes.
fn source_label(text: &str) -> String {
    let d = sha256::hex(&sha256::sha256(text.as_bytes()));
    format!("models.json#{}", &d[..8])
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

#[cfg(test)]
#[path = "collector_tests.rs"]
mod tests;
