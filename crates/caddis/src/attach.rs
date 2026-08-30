//! `caddis attach --harness …` — fail closed before any write if warden is missing.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::harness::Harness;
use crate::project;
use crate::voice;
use crate::which;

pub enum Error {
    ConscienceOffline,
    Usage(String),
    Fail(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ConscienceOffline => {
                write!(f, "CONSCIENCE OFFLINE: caddis-warden is not on PATH")
            }
            Error::Usage(s) | Error::Fail(s) => write!(f, "{s}"),
        }
    }
}

pub fn run(args: &[String]) -> Result<(), Error> {
    let parsed = parse(args)?;
    if !which::warden_on_path() {
        return Err(Error::ConscienceOffline);
    }
    let src = require_skill_src(&parsed)?;
    let home = home_dir()?;
    let dest = parsed.harness.skill_dest(&home);
    project::copy_tree(&src, &dest)
        .map_err(|e| Error::Fail(format!("project {}: {e}", dest.display())))?;
    inherit_bee(&dest)?;
    inherit_exts(&home, parsed.harness)?;
    voice::register(parsed.harness.voice_label()).map_err(Error::Fail)?;
    println!("attached {} -> {}", parsed.harness_name, dest.display());
    Ok(())
}

fn require_skill_src(parsed: &Parsed) -> Result<PathBuf, Error> {
    let src = parsed.skill_src.clone().ok_or_else(|| {
        Error::Fail("skill src missing: pass --skill-src or set CADDIS_SKILL_SRC".into())
    })?;
    if !src.join("SKILL.md").is_file() {
        return Err(Error::Fail(format!(
            "skill src has no SKILL.md: {}",
            src.display()
        )));
    }
    Ok(src)
}

struct Parsed {
    harness: Harness,
    harness_name: String,
    skill_src: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Parsed, Error> {
    let (harness_raw, mut skill_src) = parse_flags(args)?;
    let harness_name =
        harness_raw.ok_or_else(|| Error::Usage("attach requires --harness".into()))?;
    let harness = Harness::parse(&harness_name)
        .ok_or_else(|| Error::Usage(format!("unknown harness {harness_name}")))?;
    if skill_src.is_none() {
        skill_src = env::var_os("CADDIS_SKILL_SRC")
            .map(PathBuf::from)
            .or_else(find_skill_src);
    }
    Ok(Parsed {
        harness,
        harness_name,
        skill_src,
    })
}

fn parse_flags(args: &[String]) -> Result<(Option<String>, Option<PathBuf>), Error> {
    let mut harness_raw = None;
    let mut skill_src = None;
    let mut i = 0;
    while i < args.len() {
        match flag_at(args, &mut i)? {
            Flag::Harness(v) => harness_raw = Some(v),
            Flag::SkillSrc(v) => skill_src = Some(v),
        }
        i += 1;
    }
    Ok((harness_raw, skill_src))
}

enum Flag {
    Harness(String),
    SkillSrc(PathBuf),
}

fn flag_at(args: &[String], i: &mut usize) -> Result<Flag, Error> {
    if let Some(v) = next_value(args, i, "--harness")? {
        return Ok(Flag::Harness(v.to_string()));
    }
    if let Some(v) = next_value(args, i, "--skill-src")? {
        return Ok(Flag::SkillSrc(PathBuf::from(v)));
    }
    Err(Error::Usage(format!("unknown argument {}", args[*i])))
}

fn next_value<'a>(args: &'a [String], i: &mut usize, flag: &str) -> Result<Option<&'a str>, Error> {
    let a = args[*i].as_str();
    let prefix = format!("{flag}=");
    if let Some(v) = a.strip_prefix(&prefix) {
        return Ok(Some(v));
    }
    if a == flag {
        *i += 1;
        let v = args
            .get(*i)
            .ok_or_else(|| Error::Usage(format!("missing {flag} value")))?;
        return Ok(Some(v.as_str()));
    }
    Ok(None)
}

fn home_dir() -> Result<PathBuf, Error> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| Error::Fail("HOME is unset".into()))
}

fn inherit_bee(dest: &Path) -> Result<(), Error> {
    let src = env::var_os("CADDIS_BEE_SRC")
        .map(PathBuf::from)
        .or_else(|| {
            home_dir().ok().map(|h| {
                h.join(".claude")
                    .join("rules")
                    .join("common")
                    .join("droid-bees-always.md")
            })
        });
    let Some(src) = src else {
        return Ok(());
    };
    if !src.is_file() {
        return Ok(());
    }
    fs::copy(&src, dest.join("droid-bees-always.md"))
        .map_err(|e| Error::Fail(format!("bee inherit: {e}")))?;
    Ok(())
}

fn find_skill_src() -> Option<PathBuf> {
    let mut dir = env::current_dir().ok()?;
    loop {
        let cand = dir.join("skills").join("caddis");
        if cand.join("SKILL.md").is_file() {
            return Some(cand);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Projected OMP extensions: (file, env override). One walker, one law:
/// deploy is organ-owned — a hand copy into ~/.omp is the fragility the
/// operator banned (CARD-0154/0158).
const EXTENSIONS: &[(&str, &str)] = &[
    ("caddis-fold.ts", "CADDIS_FOLD_EXT"),
    ("caddis-page-observe.ts", "CADDIS_PAGE_EXT"),
    ("caddis-page.ts", "CADDIS_PAGE_NERVE_EXT"),
];

fn inherit_exts(home: &Path, harness: Harness) -> Result<(), Error> {
    if harness != Harness::OmpPeleda {
        return Ok(());
    }
    let dest_dir = home.join(".omp").join("agent").join("extensions");
    fs::create_dir_all(&dest_dir).map_err(|e| Error::Fail(format!("mkdir extensions: {e}")))?;
    for (file, env_name) in EXTENSIONS {
        if let Some(src) = ext_src(file, env_name) {
            fs::copy(&src, dest_dir.join(file))
                .map_err(|e| Error::Fail(format!("copy {file}: {e}")))?;
        }
    }
    Ok(())
}

fn ext_src(file: &str, env_name: &str) -> Option<PathBuf> {
    if let Some(p) = env::var_os(env_name) {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let want = format!("extension/{file}");
    let mut dir = env::current_dir().ok()?;
    loop {
        let cand = dir.join(&want);
        if cand.is_file() {
            return Some(cand);
        }
        if !dir.pop() {
            return None;
        }
    }
}
