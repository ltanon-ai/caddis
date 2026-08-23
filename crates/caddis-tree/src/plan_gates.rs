//! plan_gates.rs — the walker-side plan oracle (BC3): repo-reality checks
//! the crate validator deliberately does not own. caddis-card validates
//! STRUCTURE (ids/orders/disjoint paths); here, against the actual repo:
//! every child path exists, and every named symbol is greppable in that
//! child's own paths.

use caddis_card::{Plan, PlanChild};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub child: String,
    pub what: String,
}

pub fn check(plan: &Plan, repo_root: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    for child in &plan.children {
        check_child(child, repo_root, &mut out);
    }
    out
}

fn check_child(child: &PlanChild, root: &Path, out: &mut Vec<Finding>) {
    for p in &child.paths {
        if !root.join(p).is_file() {
            out.push(Finding {
                child: child.id.clone(),
                what: format!("path missing: {p}"),
            });
        }
    }
    for sym in &child.symbols {
        let greppable = child.paths.iter().any(|p| {
            fs::read_to_string(root.join(p))
                .map(|t| t.contains(sym))
                .unwrap_or(false)
        });
        if !greppable {
            out.push(Finding {
                child: child.id.clone(),
                what: format!("symbol {sym} not greppable in its own paths"),
            });
        }
    }
}
