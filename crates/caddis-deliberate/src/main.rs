//! caddis-deliberate CLI — home bootstrap (P4 s1) + signed SEED path (P4 s3).
//!
//! `seed` — create the home's card stream ONCE from the desktop catalog
//!           (idempotent on identical bytes; NEVER overwrites a diverged
//!           stream — edits ride the warden-gated path), then prove the
//!           cached view against the stream digest (F2).
//! `view` — load + sync the view against the stream truth and print the
//!           view JSON verbatim on stdout: that JSON is the machine
//!           surface the world's bridge (P4 slice 2) reads, so stdout
//!           stays PURE JSON — every human word goes to stderr.
//! `export` — sign the home's stream as a SEED artifact (F13): one flat
//!           JSON object, `sig = HMAC-SHA256(seed.key, canonical)`;
//!           mints the born-once `seed.key` beside the stream at first
//!           export. Artifact to stdout (pure JSON) or `--out <file>`.
//! `verify` — the supply-chain GATE: strict parse + digest + rows +
//!           fingerprint + signature; findings name the broken law.
//! `restore` — CONSTRUCT a home from a seed, ONLY after verify is clean
//!           (tampered seed = refused, nothing written; a diverged
//!           target is never clobbered).
//! `edits` — the warden-gated registry edit path (P4 s4a): `status` (the
//!           honest journal census), `propose --op --card` (a durable
//!           pending row; the stream is NEVER touched), `confirm --id`
//!           (THE WARDEN GATE: an active card in the ledger for the
//!           confirming actor, else a refusal — nothing written), `refuse
//!           --id` (the operator's NO, journaled). Stdout is pure JSON
//!           (the P4 world-bridge surface); exit 1 = refusal (nothing
//!           written), exit 2 = defect/usage (edits.rs taxonomy).
//!
//! Defaults follow the estate home law (caddis-warden identity.rs
//! precedent): catalog `~/.pi/agent/models.json`, home
//! `~/.caddis/deliberate/`, stream `seats.jsonl`, view `seats-view.json`.
//! `USERPROFILE` wins over `HOME` (Windows); an unset profile falls back
//! to "." so the failure is VISIBLE in the path, never silent.

use caddis_deliberate::collector::{seed_once, SeedOutcome};
use caddis_deliberate::edits::{self, EditErr, EditOp};
use caddis_deliberate::json::{self, Value};
use caddis_deliberate::registry;
use std::path::PathBuf;
use std::process::ExitCode;

fn home() -> PathBuf {
    let h = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(h)
}

fn default_models() -> PathBuf {
    home().join(".pi").join("agent").join("models.json")
}

fn default_home_dir() -> PathBuf {
    home().join(".caddis").join("deliberate")
}

/// `--models <path>` (seed only) and `--home <dir>` are the whole surface;
/// anything else is a usage error, not a guess.
fn parse_paths(args: &[String], seed: bool) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let mut models = default_models();
    let mut dir = default_home_dir();
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        match flag {
            "--models" if seed => {
                i += 1;
                models = take_value(args, i, "--models")?;
            }
            "--home" => {
                i += 1;
                dir = take_value(args, i, "--home")?;
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
        i += 1;
    }
    Ok((models, dir.join("seats.jsonl"), dir.join("seats-view.json")))
}

fn take_value(args: &[String], i: usize, flag: &str) -> Result<PathBuf, String> {
    args.get(i)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("seed") => cmd_seed(&args[1..]),
        Some("view") => cmd_view(&args[1..]),
        Some("export") => cmd_export(&args[1..]),
        Some("verify") => cmd_verify(&args[1..]),
        Some("restore") => cmd_restore(&args[1..]),
        Some("edits") => cmd_edits(&args[1..]),
        _ => {
            usage();
            ExitCode::from(2)
        }
    }
}

fn cmd_seed(args: &[String]) -> ExitCode {
    let (models, stream, view) = match parse_paths(args, true) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("caddis-deliberate seed: {e}");
            return ExitCode::from(2);
        }
    };
    let text = match std::fs::read_to_string(&models) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("caddis-deliberate seed: read {}: {e}", models.display());
            return ExitCode::FAILURE;
        }
    };
    match seed_once(&text, &stream, &view) {
        Ok(SeedOutcome::Created {
            rows,
            skipped,
            view_synced,
        }) => {
            let view_word = proven(view_synced);
            eprintln!(
                "seeded {} ({} rows, {} skipped, view {})",
                stream.display(),
                rows,
                skipped,
                view_word
            );
            ExitCode::SUCCESS
        }
        Ok(SeedOutcome::AlreadySeeded { rows, view_synced }) => {
            let view_word = proven(view_synced);
            eprintln!(
                "already seeded: {} ({} rows unchanged — idempotent; view {view_word})",
                stream.display(),
                rows
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("caddis-deliberate seed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_view(args: &[String]) -> ExitCode {
    let (_, stream, view) = match parse_paths(args, false) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("caddis-deliberate view: {e}");
            return ExitCode::from(2);
        }
    };
    match registry::load_and_sync(&stream, &view) {
        Ok(_) => match std::fs::read_to_string(&view) {
            Ok(v) => {
                println!("{v}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("caddis-deliberate view: read {}: {e}", view.display());
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("caddis-deliberate view: {}: {e}", stream.display());
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------------------
// edits — the warden-gated registry edit path (P4 s4a)
// ---------------------------------------------------------------------------

/// Everything the `edits` verbs need. Paths derive from `--home <dir>`
/// (default the organ home); the warden ledger is ESTATE-level and defaults
/// to `~/.caddis/warden-ledger.jsonl` regardless of `--home` (a sandbox home
/// still gates against the real ledger unless `--warden` says otherwise).
/// Identity law (edits.rs F2): `--actor`/`--actor-kind` arrive from the
/// CALLING TRANSPORT — default "terminal", the same self-naming convention
/// as the router author CLI; the organ never invents identity.
struct EditsArgs {
    stream: PathBuf,
    view: PathBuf,
    journal: PathBuf,
    warden: PathBuf,
    actor: String,
    actor_kind: String,
    op_word: Option<String>,
    card: Option<String>,
    id: Option<String>,
}

fn take_str(args: &[String], i: usize, flag: &str) -> Result<String, String> {
    args.get(i)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn parse_edits_args(args: &[String]) -> Result<EditsArgs, String> {
    let mut dir = default_home_dir();
    let mut warden = home().join(".caddis").join("warden-ledger.jsonl");
    let (mut actor, mut actor_kind) = ("terminal".to_string(), "terminal".to_string());
    let (mut op_word, mut card, mut id) = (None, None, None);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--home" => {
                i += 1;
                dir = take_value(args, i, "--home")?;
            }
            "--warden" => {
                i += 1;
                warden = take_value(args, i, "--warden")?;
            }
            "--actor" => {
                i += 1;
                actor = take_str(args, i, "--actor")?;
            }
            "--actor-kind" => {
                i += 1;
                actor_kind = take_str(args, i, "--actor-kind")?;
            }
            "--op" => {
                i += 1;
                op_word = Some(take_str(args, i, "--op")?);
            }
            "--card" => {
                i += 1;
                card = Some(take_str(args, i, "--card")?);
            }
            "--id" => {
                i += 1;
                id = Some(take_str(args, i, "--id")?);
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
        i += 1;
    }
    Ok(EditsArgs {
        stream: dir.join("seats.jsonl"),
        view: dir.join("seats-view.json"),
        journal: dir.join("edits.jsonl"),
        warden,
        actor,
        actor_kind,
        op_word,
        card,
        id,
    })
}

fn jstr(s: &str) -> Value {
    Value::Str(s.to_string())
}

fn jnum(n: u64) -> Value {
    Value::Num(n as f64)
}

fn jobj(pairs: Vec<(&str, Value)>) -> Value {
    Value::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// Stdout law (view precedent): machine verbs print PURE JSON; every human
/// word goes to stderr.
fn print_json(v: &Value) {
    println!("{}", json::to_string(v));
}

/// edits.rs taxonomy law: a REFUSAL is exit 1 (nothing was written — the
/// honest stop), a Defect is exit 2 (fail closed). The JSON body carries
/// the message so the world bridge can surface it without parsing stderr.
fn edit_err_exit(verb: &str, e: EditErr) -> ExitCode {
    let refusal = e.is_refusal();
    print_json(&jobj(vec![
        ("ok", Value::Bool(false)),
        ("error", jstr(&e.to_string())),
    ]));
    eprintln!("caddis-deliberate edits {verb}: {e}");
    ExitCode::from(if refusal { 1 } else { 2 })
}

fn edits_usage_exit(verb: &str, what: &str) -> ExitCode {
    eprintln!("caddis-deliberate edits {verb}: {what}");
    ExitCode::from(2)
}

fn cmd_edits(args: &[String]) -> ExitCode {
    let Some(verb) = args.first().map(|s| s.as_str()) else {
        return edits_usage_exit("edits", "missing verb (status|propose|confirm|refuse)");
    };
    match verb {
        "status" => cmd_edits_status(&args[1..]),
        "propose" => cmd_edits_propose(&args[1..]),
        "confirm" => cmd_edits_confirm(&args[1..]),
        "refuse" => cmd_edits_refuse(&args[1..]),
        other => edits_usage_exit(
            "edits",
            &format!("unknown verb {other:?} (status|propose|confirm|refuse)"),
        ),
    }
}

/// The honest journal census — read-only, never fails: rc 0 even when the
/// journal is absent (empty census, `exists:false`).
fn cmd_edits_status(args: &[String]) -> ExitCode {
    let a = match parse_edits_args(args) {
        Ok(a) => a,
        Err(e) => return edits_usage_exit("status", &e),
    };
    let st = edits::status(&a.journal);
    let pending = st
        .pending
        .iter()
        .map(|p| {
            jobj(vec![
                ("id", jstr(&p.id)),
                ("seq", jnum(p.seq)),
                ("op", jstr(p.op.op_word())),
                ("card", jstr(&registry::encode_card(&p.op.to_card()))),
                ("prior16", jstr(&p.prior16)),
                ("actor", jstr(&p.actor)),
                ("actor_kind", jstr(&p.actor_kind)),
                ("state", jstr("pending")),
                ("resolved_by", Value::Null),
            ])
        })
        .collect();
    print_json(&jobj(vec![
        ("version", jstr("1")),
        ("journal", jstr(&a.journal.display().to_string())),
        ("exists", Value::Bool(a.journal.exists())),
        ("max_seq", jnum(st.max_seq)),
        ("pending", Value::Arr(pending)),
        ("confirmed", jnum(st.confirmed as u64)),
        ("refused", jnum(st.refused as u64)),
        (
            "unparseable",
            Value::Arr(st.unparseable.iter().map(|l| jnum(*l as u64)).collect()),
        ),
    ]));
    ExitCode::SUCCESS
}

/// PROPOSE an edit: `--op <upsert-seat|upsert-provider>` + `--card <line|@file>`.
/// The card is parsed by the ONE card parser (registry::parse_stream —
/// parse law), exactly one card; the op-word×class pair is checked by
/// [`EditOp::from_parts`]. Refuses no-ops; NEVER touches the stream.
fn cmd_edits_propose(args: &[String]) -> ExitCode {
    let a = match parse_edits_args(args) {
        Ok(a) => a,
        Err(e) => return edits_usage_exit("propose", &e),
    };
    let Some(op_word) = a.op_word.as_deref() else {
        return edits_usage_exit("propose", "--op <upsert-seat|upsert-provider> is required");
    };
    let Some(card_arg) = a.card.as_deref() else {
        return edits_usage_exit("propose", "--card <encoded-card-line|@file> is required");
    };
    let card_text = match card_arg.strip_prefix('@') {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => return edits_usage_exit("propose", &format!("read {path}: {e}")),
        },
        None => card_arg.to_string(),
    };
    let cards = match registry::parse_stream(card_text.trim()) {
        Ok(c) => c,
        Err(e) => return edits_usage_exit("propose", &format!("--card does not parse: {e}")),
    };
    if cards.len() != 1 {
        return edits_usage_exit(
            "propose",
            &format!("--card holds {} cards, exactly 1 required", cards.len()),
        );
    }
    let op = match EditOp::from_parts(op_word, cards[0].clone()) {
        Ok(op) => op,
        Err(e) => return edits_usage_exit("propose", &e),
    };
    match edits::propose(&a.stream, &a.journal, op, &a.actor, &a.actor_kind) {
        Ok(proposal_id) => {
            print_json(&jobj(vec![
                ("ok", Value::Bool(true)),
                ("proposal_id", jstr(&proposal_id)),
                ("op", jstr(op_word)),
                ("actor", jstr(&a.actor)),
                ("actor_kind", jstr(&a.actor_kind)),
            ]));
            eprintln!("proposed {proposal_id} ({op_word}) — pending operator confirm");
            ExitCode::SUCCESS
        }
        Err(e) => edit_err_exit("propose", e),
    }
}

/// OPERATOR-CONFIRM: THE WARDEN GATE reads the ledger `--warden` names
/// (default the estate ledger). An ABSENT ledger is empty text — the honest
/// GateClosed refusal, never an invented pass; an UNREADABLE one (io error)
/// is a defect (fail closed: an answer that cannot be proven must not look
/// like "no").
fn cmd_edits_confirm(args: &[String]) -> ExitCode {
    let a = match parse_edits_args(args) {
        Ok(a) => a,
        Err(e) => return edits_usage_exit("confirm", &e),
    };
    let Some(id) = a.id.as_deref() else {
        return edits_usage_exit("confirm", "--id <proposal_id> is required");
    };
    let warden_text = match std::fs::read_to_string(&a.warden) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return edits_usage_exit("confirm", &format!("read {}: {e}", a.warden.display())),
    };
    match edits::confirm(
        &a.stream,
        &a.view,
        &a.journal,
        id,
        &a.actor,
        &a.actor_kind,
        &warden_text,
    ) {
        Ok(out) => {
            print_json(&jobj(vec![
                ("ok", Value::Bool(true)),
                ("proposal_id", jstr(&out.proposal_id)),
                ("confirm_seq", jnum(out.confirm_seq)),
                ("applied_key", jstr(&out.applied_key)),
                ("warden_card", jstr(&out.warden_card)),
            ]));
            eprintln!(
                "confirmed {} -> {} (warden card {})",
                out.proposal_id, out.applied_key, out.warden_card
            );
            ExitCode::SUCCESS
        }
        Err(e) => edit_err_exit("confirm", e),
    }
}

/// REFUSE: the operator's explicit NO — journaled, so the pending queue
/// stays honest (MV13 durable: resolved, not dropped).
fn cmd_edits_refuse(args: &[String]) -> ExitCode {
    let a = match parse_edits_args(args) {
        Ok(a) => a,
        Err(e) => return edits_usage_exit("refuse", &e),
    };
    let Some(id) = a.id.as_deref() else {
        return edits_usage_exit("refuse", "--id <proposal_id> is required");
    };
    match edits::refuse(&a.journal, id, &a.actor, &a.actor_kind) {
        Ok(refuse_seq) => {
            print_json(&jobj(vec![
                ("ok", Value::Bool(true)),
                ("proposal_id", jstr(id)),
                ("refuse_seq", jnum(refuse_seq)),
            ]));
            eprintln!("refused {id} (journal seq {refuse_seq})");
            ExitCode::SUCCESS
        }
        Err(e) => edit_err_exit("refuse", e),
    }
}

/// Shared flags for the seed-artifact verbs: `--home <dir>` (default the
/// organ home) or `--key <file>` (the carry-the-key path); anything else
/// is a usage error, not a guess.
fn parse_seed_args(args: &[String]) -> Result<(PathBuf, Option<PathBuf>), String> {
    let mut dir = default_home_dir();
    let mut key: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--home" => {
                i += 1;
                dir = take_value(args, i, "--home")?;
            }
            "--key" => {
                i += 1;
                key = Some(take_value(args, i, "--key")?);
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
        i += 1;
    }
    Ok((dir, key))
}

fn seed_key_path(dir: &std::path::Path, key: &Option<PathBuf>) -> PathBuf {
    key.clone().unwrap_or_else(|| dir.join("seed.key"))
}

fn cmd_export(args: &[String]) -> ExitCode {
    let mut dir = default_home_dir();
    let mut out: Option<PathBuf> = None;
    let mut i = 0;
    let args = {
        // `--out` is export-only; reuse the shared parser for the rest.
        let mut rest = Vec::new();
        while i < args.len() {
            if args[i] == "--out" {
                i += 1;
                match args.get(i) {
                    Some(v) => out = Some(PathBuf::from(v)),
                    None => {
                        eprintln!("caddis-deliberate export: --out needs a value");
                        return ExitCode::from(2);
                    }
                }
            } else {
                rest.push(args[i].clone());
            }
            i += 1;
        }
        rest
    };
    if let Err(e) = (|| {
        let (d, _) = parse_seed_args(&args)?;
        dir = d;
        Ok::<(), String>(())
    })() {
        eprintln!("caddis-deliberate export: {e}");
        return ExitCode::from(2);
    }
    match caddis_deliberate::seed::export_seed(&dir) {
        Ok(ex) => {
            let minted = if ex.key_minted {
                "key MINTED (born once)"
            } else {
                "key reused"
            };
            eprintln!(
                "exported seed: {} rows, stream {}, fingerprint {}, {minted}",
                ex.rows,
                &ex.stream_sha256[..16],
                ex.fingerprint
            );
            match out {
                Some(path) => match std::fs::write(&path, ex.artifact.as_bytes()) {
                    Ok(_) => {
                        eprintln!("artifact written to {}", path.display());
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("caddis-deliberate export: write {}: {e}", path.display());
                        ExitCode::FAILURE
                    }
                },
                None => {
                    print!("{}", ex.artifact);
                    ExitCode::SUCCESS
                }
            }
        }
        Err(e) => {
            eprintln!("caddis-deliberate export: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_verify(args: &[String]) -> ExitCode {
    let (artifact, rest) = match args.split_first() {
        Some((a, r)) if !a.starts_with("--") => (PathBuf::from(a), r.to_vec()),
        _ => {
            eprintln!("caddis-deliberate verify: needs an artifact path");
            return ExitCode::from(2);
        }
    };
    let (dir, key) = match parse_seed_args(&rest) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("caddis-deliberate verify: {e}");
            return ExitCode::from(2);
        }
    };
    let text = match std::fs::read_to_string(&artifact) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("caddis-deliberate verify: read {}: {e}", artifact.display());
            return ExitCode::FAILURE;
        }
    };
    let slot = caddis_deliberate::seed::SeedKeySlot::load(&seed_key_path(&dir, &key));
    let verdict = caddis_deliberate::seed::verify_seed_text(&text, &slot);
    if verdict.clean {
        eprintln!("seed VERIFIED: {verdict}");
        ExitCode::SUCCESS
    } else {
        eprintln!("seed REFUSED: {verdict}");
        ExitCode::from(4)
    }
}

fn cmd_restore(args: &[String]) -> ExitCode {
    let (artifact, rest) = match args.split_first() {
        Some((a, r)) if !a.starts_with("--") => (PathBuf::from(a), r.to_vec()),
        _ => {
            eprintln!("caddis-deliberate restore: needs an artifact path");
            return ExitCode::from(2);
        }
    };
    let mut to: Option<PathBuf> = None;
    let mut shared = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "--to" {
            i += 1;
            match rest.get(i) {
                Some(v) => to = Some(PathBuf::from(v)),
                None => {
                    eprintln!("caddis-deliberate restore: --to needs a value");
                    return ExitCode::from(2);
                }
            }
        } else {
            shared.push(rest[i].clone());
        }
        i += 1;
    }
    let Some(to) = to else {
        eprintln!("caddis-deliberate restore: --to <dir> is required");
        return ExitCode::from(2);
    };
    let (dir, key) = match parse_seed_args(&shared) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("caddis-deliberate restore: {e}");
            return ExitCode::from(2);
        }
    };
    let text = match std::fs::read_to_string(&artifact) {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "caddis-deliberate restore: read {}: {e}",
                artifact.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let slot = caddis_deliberate::seed::SeedKeySlot::load(&seed_key_path(&dir, &key));
    match caddis_deliberate::seed::restore_seed(&text, &slot, &to) {
        Ok(caddis_deliberate::seed::RestoreOutcome::Constructed { rows }) => {
            eprintln!(
                "home CONSTRUCTED at {} ({rows} rows, view proven)",
                to.display()
            );
            ExitCode::SUCCESS
        }
        Ok(caddis_deliberate::seed::RestoreOutcome::AlreadyIdentical { rows }) => {
            eprintln!("target already identical ({rows} rows, view proven) — idempotent");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("caddis-deliberate restore: {e}");
            ExitCode::from(4)
        }
    }
}

fn proven(view_synced: bool) -> &'static str {
    if view_synced {
        "proven (rewritten)"
    } else {
        "already current"
    }
}

fn usage() {
    eprintln!(
        "usage: caddis-deliberate seed [--models <catalog.json>] [--home <dir>]\n       \
         caddis-deliberate view [--home <dir>]\n       \
         caddis-deliberate export [--home <dir>] [--out <artifact.json>]\n       \
         caddis-deliberate verify <artifact.json> [--home <dir> | --key <file>]\n       \
         caddis-deliberate restore <artifact.json> --to <dir> [--home <dir> | --key <file>]\n       \
         caddis-deliberate edits status  [--home <dir>] [--warden <path>]\n       \
         caddis-deliberate edits propose --op <upsert-seat|upsert-provider> --card <line|@file>\n       \
                                [--actor <name>] [--actor-kind <word>] [--home <dir>]\n       \
         caddis-deliberate edits confirm --id <eN> [--actor <name>] [--actor-kind <word>]\n       \
                                [--warden <path>] [--home <dir>]\n       \
         caddis-deliberate edits refuse  --id <eN> [--actor <name>] [--actor-kind <word>] [--home <dir>]\n       \
         defaults: catalog ~/.pi/agent/models.json, home ~/.caddis/deliberate,\n       \
         warden ledger ~/.caddis/warden-ledger.jsonl (the confirm gate)\n       \
         exit codes: 0 ok, 1 io error or edits REFUSAL (nothing written), 2 usage/defect,\n       \
         4 seed REFUSED (verify gate)"
    );
}

#[cfg(test)]
#[path = "main_edits_tests.rs"]
mod main_edits_tests;
