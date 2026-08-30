//! occupancy.rs — CARD-0333. Muhstation GPU occupancy.
//!
//! Operator switches modes MANUALLY (`/bee on|off` or the panel). This
//! organ never writes the panel. Dark KAT lanes while occupied is NOT
//! an error. Coding cards: station bees when mode=bee; else fallback
//! droid-glm then commandcode-deepseek. Never ollama.

use std::fs;
use std::path::{Path, PathBuf};

pub enum Error {
    Usage(String),
    Fail(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(s) | Self::Fail(s) => write!(f, "{s}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Bee,
    Occupied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodingRoute {
    Station,
    Fallback,
}

/// Cheap code hands when the station GPU is the operator's, not ours.
pub const FALLBACK: &[&str] = &["droid-glm", "commandcode-deepseek"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occupancy {
    pub mode: Mode,
    pub station: String,
    pub missing: bool,
}

impl Occupancy {
    pub fn coding_route(&self) -> CodingRoute {
        match self.mode {
            Mode::Bee => CodingRoute::Station,
            Mode::Occupied => CodingRoute::Fallback,
        }
    }
}

fn parse_mode(v: &str) -> Result<Mode, String> {
    match v.trim() {
        "bee" => Ok(Mode::Bee),
        "occupied" => Ok(Mode::Occupied),
        other => Err(format!("unknown mode {other}")),
    }
}

fn apply_line(mode: &mut Option<Mode>, station: &mut String, line: &str) -> Result<(), String> {
    if let Some(v) = line.strip_prefix("mode=") {
        *mode = Some(parse_mode(v)?);
        return Ok(());
    }
    if let Some(v) = line.strip_prefix("station=") {
        *station = v.trim().to_string();
        return Ok(());
    }
    Err(format!("unknown field {line}"))
}

pub fn parse(text: &str) -> Result<Occupancy, String> {
    let mut mode = None;
    let mut station = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        apply_line(&mut mode, &mut station, line)?;
    }
    let mode = mode.ok_or_else(|| "missing mode=".to_string())?;
    if station.is_empty() {
        station = match mode {
            Mode::Bee => "bee".into(),
            Mode::Occupied => "other".into(),
        };
    }
    Ok(Occupancy {
        mode,
        station,
        missing: false,
    })
}

pub fn load(path: &Path) -> Result<Occupancy, String> {
    match fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Occupancy {
            mode: Mode::Occupied,
            station: "unknown".into(),
            missing: true,
        }),
        Err(e) => Err(format!("read {}: {e}", path.display())),
    }
}

pub fn default_path() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".caddis").join("occupancy")
}

fn take_file(args: &[String]) -> Result<PathBuf, Error> {
    let mut file = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--help" {
            return Err(Error::Usage(
                "usage: caddis occupancy [--file PATH]".into(),
            ));
        }
        if args[i] != "--file" {
            return Err(Error::Usage(format!("unknown argument {}", args[i])));
        }
        i += 1;
        let v = args
            .get(i)
            .ok_or_else(|| Error::Usage("missing --file value".into()))?;
        file = Some(PathBuf::from(v));
        i += 1;
    }
    Ok(file.unwrap_or_else(default_path))
}

pub fn run(args: &[String]) -> Result<i32, Error> {
    let path = take_file(args)?;
    let occ = load(&path).map_err(Error::Fail)?;
    let coding = match occ.coding_route() {
        CodingRoute::Station => "station",
        CodingRoute::Fallback => "fallback",
    };
    let miss = if occ.missing { " missing=1" } else { "" };
    let mode = match occ.mode {
        Mode::Bee => "bee",
        Mode::Occupied => "occupied",
    };
    println!(
        "occupancy {mode} station={} coding={coding} fallback={}{miss}",
        occ.station,
        FALLBACK.join(",")
    );
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occupied_is_ok_not_error() {
        let o = parse("mode=occupied\nstation=benchmark\n").unwrap();
        assert_eq!(o.mode, Mode::Occupied);
        assert_eq!(o.coding_route(), CodingRoute::Fallback);
        assert_eq!(o.station, "benchmark");
    }

    #[test]
    fn bee_routes_to_station() {
        let o = parse("mode=bee\n").unwrap();
        assert_eq!(o.mode, Mode::Bee);
        assert_eq!(o.coding_route(), CodingRoute::Station);
        assert_eq!(o.station, "bee");
    }

    #[test]
    fn fallback_never_names_ollama() {
        for id in FALLBACK {
            assert!(!id.contains("ollama"), "{id}");
        }
        assert_eq!(FALLBACK, &["droid-glm", "commandcode-deepseek"]);
    }

    #[test]
    fn unknown_mode_is_malformed() {
        assert!(parse("mode=wedge\n").is_err());
    }

    #[test]
    fn missing_mode_is_malformed() {
        assert!(parse("station=bee\n").is_err());
    }
}
