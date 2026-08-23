//! shared fixtures for the BC3 tree tests (CARD-0093).

#![allow(dead_code)]

use caddis_card::Card;
use caddis_tree::state::{Caps, TreeState};
use caddis_tree::walker::{Outcome, SimExecutor, Walker};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);

pub fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "caddis-tree-{}-{}-{}",
        name,
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    fs::create_dir_all(&d).unwrap();
    d
}

pub fn seed_repo(root: &Path) {
    fs::write(
        root.join("root_red.md"),
        "# root integration RED\nassert!(tree_works());\n",
    )
    .unwrap();
    fs::write(root.join("a.py"), "def foo():\n    return 1\n").unwrap();
    fs::write(root.join("b.py"), "def bar():\n    return 2\n").unwrap();
}

pub fn plan() -> caddis_card::Plan {
    let text = "---\nid: PLAN-T\nclass: plan\nowner: t\n---\n\n# T\n\n\
# Done-When\n\n- both children land\n\n# RED-TEST\n\n- nothing walked yet\n\n\
# CHILDREN\n\n- id: CARD-A\n  order: 1\n  paths: a.py\n  symbols: foo\n\
- id: CARD-B\n  order: 2\n  paths: b.py\n  symbols: bar\n\n\
# REVIEW\n\nreviewer: strong-lane\nverdict: accepted\nchecks: seams match\n";
    Card::parse(text).unwrap().validate_plan().unwrap()
}

pub fn caps() -> Caps {
    Caps {
        max_attempts: 10,
        max_cost: 1000,
    }
}

pub fn walking(root: &Path) -> Walker {
    let st = TreeState::new(root.join("goal.jsonl"), "w1", caps()).unwrap();
    let mut w = Walker::new(st, root.to_path_buf());
    w.intake("root_red.md").unwrap();
    w
}

pub fn pass_exec() -> SimExecutor {
    SimExecutor::new(vec![Outcome {
        pass: true,
        cost: 5,
    }])
}

pub fn fail_exec() -> SimExecutor {
    SimExecutor::new(vec![Outcome {
        pass: false,
        cost: 5,
    }])
}
