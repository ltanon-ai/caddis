//! cli.rs — the human-facing arguments, kept off the frame path.
//!
//! WHY THIS EXISTS. Every invocation used to fall through to "read a request
//! frame from stdin", so `caddis-warden --version` answered with a DENIAL and
//! exited 0: the first command a new user types reported that their install had
//! refused them, while claiming success. A conscience whose own CLI cannot say
//! what it is has no standing to say what it judged.
//!
//! THE FRAME PATH IS DELIBERATELY UNTOUCHED. Adapters spawn the binary with NO
//! arguments and read the verdict from stdout; the Claude Code adapter ignores
//! the exit code entirely, so the deny-on-unreadable contract is unchanged and
//! a denial still exits 0.

pub const USAGE: &str = "\
caddis-warden — the conscience a harness calls once per tool call.

USAGE:
  caddis-warden                     judge one request frame read from stdin
                                    (what an adapter does; wire: PROTOCOL.md)
  caddis-warden --replay <ledger.jsonl> [--from NAME] [--since HOURS]
                                    re-judge recorded history against today's law
  caddis-warden report [--from NAME] [--since HOURS] [--json]
                                    summarise what the ledger recorded
  caddis-warden receipt [--from NAME] [--since HOURS] [--json]
                                    what one caller did, reconstructed from the ledger
  caddis-warden laws [--from NAME] [--since HOURS] [--json]
                                    which laws EARN their place, are WALLPAPER, or are DEAD
  caddis-warden propose-laws [--since HOURS] [--json]
                                    candidate laws mined from allow-then-undo history
  caddis-warden card open <card.md> declare a card open for THIS session
  caddis-warden card status         which card this session holds open
  caddis-warden card close [--verify -- <cmd>]  close; --verify runs cmd first
  caddis-warden ledger rotate       archive the live ledger; never rewrite it
  caddis-warden attest --card <CARD-ID> [--json]
                                    a proof bundle for one card, from the ledger
  caddis-warden attest --verify <bundle.json>
                                    re-check a bundle against the ledger
  caddis-warden --version           print the version
  caddis-warden --help              print this

`--from NAME` matches a lane and every session in it: `--from peleda` selects
`peleda` and `peleda.a1b2c3d4`, but never a different lane called `peleda-two`.

A card is state DERIVED FROM THE LEDGER, so it survives this process, is visible
to `report`, and cannot drift from the record the way a side file would. It is
per SESSION: opening a card requires an adapter that stamps
`CADDIS_WARDEN_FROM=<label>.<session>`, because a card held under a bare harness
label would bound a different session's writes.

The frame path writes one JSON verdict to stdout and exits 0 even for a denial:
adapters read the verdict, never the exit code. Installing: ONBOARD.md.";

/// What to do with an argument, decided without ever touching stdin.
pub enum Reply {
    /// Write to stdout and exit 0.
    Out(String),
    /// Write to stderr and exit with this code.
    Err(String, i32),
}

/// Answer one leading argument. `--replay` and `report` never arrive here:
/// `main` dispatches those subcommands before asking, and that ordering is the
/// contract — this function is total for everything else.
pub fn for_flag(arg: &str) -> Reply {
    match arg {
        "--version" | "-V" => Reply::Out(version_line()),
        "--help" | "-h" => Reply::Out(USAGE.to_string()),
        other => Reply::Err(
            format!("caddis-warden: unknown argument `{other}`\n\n{USAGE}"),
            2,
        ),
    }
}

/// A downloaded binary must be able to say WHICH release it is.
///
/// The crate version alone cannot: crates keep their own versions while the
/// product version lives in the tag (TWIN-REPO-DOCTRINE), so someone holding
/// the v0.2.x download would be told "0.1.0" and reasonably conclude they had
/// the wrong file. The release build stamps `CADDIS_RELEASE`; every other build
/// says plainly that it is not one, rather than implying a release it is not.
pub fn version_line() -> String {
    let crate_version = env!("CARGO_PKG_VERSION");
    match option_env!("CADDIS_RELEASE") {
        Some(release) if !release.is_empty() => {
            format!("caddis-warden {crate_version} (release {release})")
        }
        _ => format!("caddis-warden {crate_version} (unreleased build)"),
    }
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
