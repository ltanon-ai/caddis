//! Chair kinds `caddis attach --harness` understands.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Harness {
    OmpPeleda,
    Claude,
    Qpi,
}

impl Harness {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "omp-peleda" => Some(Self::OmpPeleda),
            "claude" => Some(Self::Claude),
            "qpi" => Some(Self::Qpi),
            _ => None,
        }
    }

    pub fn skill_dest(self, home: &Path) -> PathBuf {
        match self {
            Self::OmpPeleda => home
                .join(".omp")
                .join("agent")
                .join("skills")
                .join("caddis"),
            Self::Claude => home.join(".claude").join("skills").join("caddis"),
            Self::Qpi => home
                .join(".qpi")
                .join("agent")
                .join("skills")
                .join("caddis"),
        }
    }

    pub fn voice_label(self) -> &'static str {
        match self {
            Self::OmpPeleda => "OMP Pelėda",
            Self::Claude => "Pelėda",
            Self::Qpi => "QPI",
        }
    }
}
