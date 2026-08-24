"""test_sonar_coverage.py — the coverage pipeline's own tests.

sonar-coverage.py is new code under the 80% new-coverage gate; these
tests exercise both transforms on tiny fixtures so the pipeline proves
itself the way it proves everything else.
"""

import importlib.util
import os

_HERE = os.path.dirname(os.path.abspath(__file__))
_SPEC = importlib.util.spec_from_file_location(
    "sonar_coverage", os.path.join(_HERE, "sonar-coverage.py")
)
assert _SPEC is not None and _SPEC.loader is not None
sc = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(sc)

COBERTURA_SAMPLE = """<coverage version="1">
  <sources>
    <source>{src}</source>
  </sources>
  <packages>
    <package name="." line-rate="0.5">
      <classes>
        <class name="mod" filename="mod.py" line-rate="0.5">
          <lines>
            <line number="1" hits="1"/>
            <line number="2" hits="0"/>
          </lines>
        </class>
      </classes>
    </package>
  </packages>
</coverage>
"""

LCOV_SAMPLE = """SF:{root}\\crates\\x\\src\\lib.rs
DA:1,1
DA:2,0
end_of_record
"""


def test_to_relative_strips_cwd_and_scanner_root(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    root = str(tmp_path).replace("\\", "/")
    assert sc.to_relative(root + "/crates/x.rs") == "crates/x.rs"
    assert sc.to_relative("/usr/src/crates/x.rs") == "crates/x.rs"
    assert sc.to_relative("crates/x.rs") == "crates/x.rs"


def test_bare_filenames_resolve_against_the_real_source(tmp_path, monkeypatch):
    src = tmp_path / "pkg_a"
    src.mkdir()
    (src / "mod.py").write_text("x = 1\n", encoding="utf-8")
    xml = tmp_path / "coverage-python.xml"
    xml.write_text(COBERTURA_SAMPLE.format(src=src), encoding="utf-8")
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(sc, "PY_XML", str(xml))
    sc.fix_cobertura_sources()
    text = xml.read_text(encoding="utf-8")
    assert "<source>.</source>" in text
    assert 'filename="pkg_a/mod.py"' in text, "bare name resolved via the source dir"


def test_lcov_becomes_generic_with_relative_paths(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    lcov = tmp_path / "coverage-rust.lcov"
    lcov.write_text(LCOV_SAMPLE.format(root=tmp_path), encoding="utf-8")
    out = tmp_path / "generic.xml"
    monkeypatch.setattr(sc, "LCOV_IN", str(lcov))
    monkeypatch.setattr(sc, "GENERIC_OUT", str(out))
    sc.lcov_to_generic()
    import xml.etree.ElementTree as ET

    root = ET.parse(out).getroot()
    assert root.tag == "coverage"
    assert root[0].get("path") == "crates/x/src/lib.rs"
    lines = root[0].findall("lineToCover")
    assert lines[0].get("covered") == "true" and lines[1].get("covered") == "false"
