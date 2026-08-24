"""sonar-coverage.py — prepare real coverage for the SonarQube scan.

One entry point for the pre-push coverage pipeline (operator ruling
2026-08-24, option 1). Prerequisites: `pip install pytest pytest-cov
defusedxml`, `cargo llvm-cov` (with llvm-tools-preview):

    python -m pytest -q skills/caddis/test_ladder.py \
        adapters/claude-code/test_warden_gate.py tools/test_sonar_coverage.py \
        tools/test_assets.py \
        --cov=skills/caddis --cov=adapters/claude-code --cov=tools \
        --cov-report=xml:.sonar-scan/coverage-python.xml
    cargo llvm-cov --lcov --output-path .sonar-scan/coverage-rust.lcov
Both reports are produced on the host:
1. cobertura XML: rewrite the host-absolute <source> elements to
   project-relative paths so the sensor can resolve the files at
   /usr/src (otherwise the coverage is silently ignored).
2. lcov: convert to SonarQube's GENERIC coverage XML — the only third
   format sonar.coverageReportPaths accepts — with project-relative
   paths.
"""
import os
from defusedxml import ElementTree as SafeET

PY_XML = ".sonar-scan/coverage-python.xml"
LCOV_IN = ".sonar-scan/coverage-rust.lcov"
GENERIC_OUT = ".sonar-scan/coverage-rust-generic.xml"


def _repo_relative(filename: str, sources: list, here: str, package: str) -> str:
    """One cobertura class filename, resolved against the <source>
    directories (and the Cobertura package path) that actually contain
    it on disk, then made project-relative for the scanner's /usr/src
    view. Nested packages (assets under tools) resolve through their
    package segment, not only the source root; a source that IS the
    project root yields no "./" prefix (sonar resolves neither)."""
    fn = filename.replace("\\", "/")
    if "/" in fn and os.path.isfile(fn):
        return os.path.relpath(fn, here).replace("\\", "/")
    bare = fn.rsplit("/", 1)[-1]
    pkg = package.replace(".", "/") if package and package != "." else ""
    cands = (f"{pkg}/{bare}", bare) if pkg else (bare,)
    for s in sources:
        rel = os.path.relpath(s, here).replace("\\", "/")
        prefix = "" if rel == "." else rel + "/"
        for cand in cands:
            if os.path.isfile(os.path.join(s, cand)):
                return f"{prefix}{cand}"
    return cands[0]


def fix_cobertura_sources() -> None:
    tree = SafeET.parse(PY_XML)
    root = tree.getroot()
    srcs_el = root.find("sources")
    sources = [(s.text or "").strip() for s in (srcs_el or [])]
    here = os.getcwd()
    for pkg in root.iter("package"):
        for cls in pkg.iter("class"):
            cls.set(
                "filename",
                _repo_relative(
                    cls.get("filename") or "", sources, here, pkg.get("name") or ""
                ),
            )
    if srcs_el is not None:
        root.remove(srcs_el)
    srcs = root.makeelement("sources", {})
    src = root.makeelement("source", {})
    src.text = "."
    srcs.append(src)
    root.insert(0, srcs)
    tree.write(PY_XML, encoding="unicode", xml_declaration=True)
    print(f"fixed {PY_XML}: sources are project-relative")


def to_relative(path: str) -> str:
    p = path.replace("\\", "/")
    here = os.getcwd().replace("\\", "/").rstrip("/")
    if here and p.lower().startswith(here.lower() + "/"):
        return p[len(here) + 1:]
    i = p.lower().find("/usr/src/")
    if i >= 0:
        return p[i + len("/usr/src/"):]
    return p


def _xml_escape(text: str) -> str:
    return (
        text.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace('"', "&quot;")
    )


def lcov_to_generic() -> None:
    # Built as text, not via xml.etree builders: the safe parser
    # (defusedxml) does not re-export the builder API, and importing the
    # stdlib module trips bandit B405 even when only builders are used.
    out = ['<coverage version="1">']
    for block in open(LCOV_IN, encoding="utf-8", errors="replace").read().split(
        "end_of_record"
    ):
        lines = block.splitlines()
        sf = next((ln[3:].strip() for ln in lines if ln.startswith("SF:")), None)
        if not sf:
            continue
        out.append(f'  <file path="{_xml_escape(to_relative(sf))}">')
        for line in lines:
            if line.startswith("DA:"):
                number, hits, *_ = line[3:].split(",")
                covered = "true" if int(hits) > 0 else "false"
                out.append(
                    f'    <lineToCover lineNumber="{number}" covered="{covered}"/>'
                )
        out.append("  </file>")
    out.append("</coverage>")
    with open(GENERIC_OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(out) + "\n")
    files = sum(1 for ln in out if ln.lstrip().startswith("<file "))
    print(f"wrote {GENERIC_OUT}: {files} file(s)")

def main() -> int:
    fix_cobertura_sources()
    lcov_to_generic()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
