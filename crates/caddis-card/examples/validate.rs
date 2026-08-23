//! validate.rs — card contract runner: `cargo run -p caddis-card --example
//! validate -- <card.md>` exits 0 when the card parses and satisfies the
//! schema (frontmatter id/class/owner + Done-When + RED-TEST), 1 otherwise.
//! `--strict` adds the EXECUTION contract; `--plan` runs the PLAN oracle
//! (validate_plan: CHILDREN + REVIEW receipt) instead of strict. The
//! estate's loop demands every card be machine-checked before dispatch;
//! this is that check, one file, no ceremony.
fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: validate <card.md>");
        std::process::exit(2);
    });
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        std::process::exit(2);
    });
    let strict = std::env::args().any(|a| a == "--strict");
    let plan = std::env::args().any(|a| a == "--plan");
    match caddis_card::Card::parse(&text) {
        Ok(card) => match card.validate() {
            Ok(()) if plan => match card.validate_plan() {
                Ok(p) => {
                    println!(
                        "VALID[plan]: {} children={} verdict={}",
                        path,
                        p.children.len(),
                        p.review.verdict
                    );
                }
                Err(e) => {
                    eprintln!("INVALID[plan] {path}: {e:?}");
                    std::process::exit(1);
                }
            },
            Ok(()) if strict => match card.validate_strict() {
                Ok(exec) => {
                    println!(
                        "VALID[strict]: {} level={} blast={}",
                        path, exec.level, exec.blast
                    );
                }
                Err(e) => {
                    eprintln!("INVALID[strict] {path}: {e:?}");
                    std::process::exit(1);
                }
            },
            Ok(()) => {
                println!("VALID: {} ({} sections)", path, card.sections.len());
            }
            Err(e) => {
                eprintln!("INVALID {path}: {e:?}");
                std::process::exit(1);
            }
        },
        Err(e) => {
            eprintln!("PARSE FAIL {path}: {e:?}");
            std::process::exit(1);
        }
    }
}
