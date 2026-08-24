"""sonar-coverage.py — prepare real coverage for the SonarQube scan.

One entry point for the pre-push coverage pipeline (operator ruling
2026-08-24, option 1):

    python -m pytest -q skills/caddis/test_ladder.py \
        adapters/claude-code/test_warden_gate.py tools/test_sonar_coverage.py \
        --cov=skills/caddis --cov=adapters/claude-code --cov=tools \
        --cov-report=xml:.sonar-scan/coverage-python.xml
reports are produced on the host:

1. cobertura XML: rewrite the host-absolute <source> elements to
   project-relative paths so the sensor can resolve the files at
   /usr/src (otherwise the coverage is silently ignored).
2. lcov: convert to SonarQube's GENERIC coverage XML — the only third
   format sonar.coverageReportPaths accepts — with project-relative
   paths.
"""
import os
import xml.etree.ElementTree as ET

PY_XML = ".sonar-scan/coverage-python.xml"
LCOV_IN = ".sonar-scan/coverage-rust.lcov"
GENERIC_OUT = ".sonar-scan/coverage-rust-generic.xml"


def _repo_relative(filename: str, sources: list, here: str) -> str:
    """One cobertura class filename, resolved against the <source>
    directories that actually contain it on disk, then made
    project-relative for the scanner's /usr/src view."""
    fn = filename.replace("\\", "/")
    if "/" in fn and os.path.isfile(fn):
        return os.path.relpath(fn, here).replace("\\", "/")
    bare = fn.rsplit("/", 1)[-1]
    for s in sources:
        if os.path.isfile(os.path.join(s, bare)):
            rel = os.path.relpath(s, here).replace("\\", "/")
            return f"{rel}/{bare}"
    return bare


def fix_cobertura_sources() -> None:
    tree = ET.parse(PY_XML)
    root = tree.getroot()
    # Non-package modules (loaded by path) get a "." package and a BARE
    # class filename; the directory truth lives in the host-absolute
    # <source> elements. Resolve every class, then rewrite the XML with
    # a single project-root source.
    srcs_el = root.find("sources")
    sources = [(s.text or "").strip() for s in (srcs_el or [])]
    here = os.getcwd()
    for pkg in root.iter("package"):
        for cls in pkg.iter("class"):
            cls.set(
                "filename", _repo_relative(cls.get("filename") or "", sources, here)
            )
    if srcs_el is not None:
        root.remove(srcs_el)
    new = ET.Element("sources")
    ET.SubElement(new, "source").text = "."
    root.insert(0, new)
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


def lcov_to_generic() -> None:
    root = ET.Element("coverage", {"version": "1"})
    for block in open(LCOV_IN, encoding="utf-8", errors="replace").read().split(
        "end_of_record"
    ):
        lines = block.splitlines()
        sf = next((ln[3:].strip() for ln in lines if ln.startswith("SF:")), None)
        if not sf:
            continue
        fe = ET.SubElement(root, "file", {"path": to_relative(sf)})
        for line in lines:
            if line.startswith("DA:"):
                number, hits, *_ = line[3:].split(",")
                ET.SubElement(
                    fe,
                    "lineToCover",
                    {
                        "lineNumber": number,
                        "covered": "true" if int(hits) > 0 else "false",
                    },
                )
    ET.indent(root)
    ET.ElementTree(root).write(GENERIC_OUT, encoding="utf-8", xml_declaration=True)
    print(f"wrote {GENERIC_OUT}: {len(root)} file(s)")


def main() -> int:
    fix_cobertura_sources()
    lcov_to_generic()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
