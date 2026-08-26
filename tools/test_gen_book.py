"""test_gen_book.py — the book generator's own tests.

WHY THIS FILE EXISTS. The v0.3.0 release shipped nine SonarQube violations in
gen_book.py and the BOOK.html template it emits, and nobody saw them: the scan
on that head died parsing a stale coverage report, so the gate reported nothing
rather than reporting a failure. The fixes were verified by regenerating the
book and diffing 927 KB of output by hand — a proof that evaporates with the
session that performed it.

So the six TEMPLATE findings are pinned here as assertions rather than as a
memory. Each `test_template_*` fails if its defect is reintroduced, which is
what makes them checks rather than documentation. They are deliberately written
against the TEMPLATE STRING, not against a generated file: BOOK.html is build
output, and asserting on it would test whether someone remembered to regenerate.
"""

import importlib.util
import os
import re

_HERE = os.path.dirname(os.path.abspath(__file__))
_SPEC = importlib.util.spec_from_file_location(
    "gen_book", os.path.join(_HERE, "gen_book.py")
)
assert _SPEC is not None and _SPEC.loader is not None
gb = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(gb)


# --- the markdown-lite renderer ------------------------------------------
# chapter() was cognitive complexity 21; the rendering moved into these two
# helpers. These cases are the behaviour the split had to preserve.


def test_heading_levels_map_to_h2_h3_h4():
    assert gb._render_line("# Title") == "<h2>Title</h2>"
    assert gb._render_line("## Section") == "<h3>Section</h3>"
    assert gb._render_line("### Sub") == "<h4>Sub</h4>"


def test_a_deeper_heading_is_not_matched_by_a_shallower_rule():
    """`### x` must not render as an h2 containing `## x`."""
    assert gb._render_line("### x") == "<h4>x</h4>"
    assert gb._render_line("## x") == "<h3>x</h3>"


def test_list_items_accept_both_bullet_characters_and_indentation():
    assert gb._render_line("- one") == '<div class="li">one</div>'
    assert gb._render_line("* two") == '<div class="li">two</div>'
    assert gb._render_line("   - deep") == '<div class="li">deep</div>'


def test_table_rows_paragraphs_and_blank_lines():
    assert gb._render_line("| a | b |") == '<div class="trow">| a | b |</div>'
    assert gb._render_line("plain") == "<p>plain</p>"
    assert gb._render_line("") == "<div class='sp'></div>"
    assert gb._render_line("   ") == "<div class='sp'></div>"


def test_rendered_lines_are_html_escaped():
    """A doc containing markup must not be able to inject it into the book."""
    assert "<script>" not in gb._render_line("<script>alert(1)</script>")
    assert "&lt;script&gt;" in gb._render_line("<script>alert(1)</script>")


def test_a_mermaid_fence_passes_through_for_client_side_render():
    """Mermaid source is ESCAPED, and that is correct rather than a defect.

    The browser decodes entities while parsing, so `A--&gt;B` in the file is the
    text `A-->B` in the DOM by the time mermaid reads the element. Emitting the
    raw arrow instead would let a `<` inside a diagram break the document. This
    test asserted the unescaped form first and failed — the expectation was
    written from an assumption about the file rather than about the DOM.
    """
    out = gb._render_fence("mermaid", ["graph TD", "A-->B"])
    assert '<pre class="mermaid">' in out
    assert "graph TD\nA--&gt;B" in out
    assert '<div class="mmwrap">' in out


def test_a_plain_fence_becomes_escaped_code():
    """Presence of the escaped form is not absence of the raw one.

    A renderer that emitted both would satisfy `"&lt;" in out`, so the
    ABSENCE of the raw `<` inside the code block is the real property.
    """
    out = gb._render_fence("python", ["x = 1 < 2"])
    assert '<pre class="code"><code>' in out
    assert "1 &lt; 2" in out
    assert "1 < 2" not in out


def test_git_is_annotated_as_taking_a_list():
    """The Sonar defect WAS the annotation, and only the annotation pins it.

    Python does not enforce annotations, so no runtime call can tell
    `args: str` from `args: list[str]` — reverting the fix leaves any
    behavioural test green.
    """
    assert gb.git.__annotations__["args"] == list[str]


def test_git_passes_its_arguments_through_as_separate_argv_entries():
    """A list must EXTEND argv, never be spread character-wise.

    The first version of this test called real git and accepted `"?"` — which
    is git()'s own failure sentinel for ANY non-zero exit. So a broken argv
    left it green, and so did reverting the annotation: it asserted nothing.
    It also shelled out to whatever git was on PATH, making it the one
    environment-dependent test in a file written to kill exactly that.
    """
    seen = {}

    class _Done:
        returncode = 0
        stdout = "true\n"

    def _fake_run(argv, **kwargs):
        seen["argv"] = argv
        return _Done()

    real_run = gb.subprocess.run
    gb.subprocess.run = _fake_run
    try:
        assert gb.git(["rev-parse", "--is-inside-work-tree"]) == "true"
    finally:
        gb.subprocess.run = real_run

    assert seen["argv"] == ["git", "-C", gb.WS, "rev-parse", "--is-inside-work-tree"]


def test_git_reports_its_failure_sentinel_on_a_non_zero_exit():
    """`?` means the call FAILED. Pinned so no test may read it as success."""

    class _Failed:
        returncode = 128
        stdout = ""

    real_run = gb.subprocess.run
    gb.subprocess.run = lambda argv, **kwargs: _Failed()
    try:
        assert gb.git(["rev-parse", "nope"]) == "?"
    finally:
        gb.subprocess.run = real_run


# --- the emitted template ------------------------------------------------
# One test per SonarQube finding that reached the public repo in v0.3.0.

_FONT_SHORTHAND = re.compile(r"font:(?![a-z-]*:)[^;{}]*", re.IGNORECASE)
_GENERIC_FAMILIES = ("serif", "sans-serif", "monospace", "cursive", "fantasy")


def test_template_every_font_shorthand_names_a_generic_family():
    """Web:S5723 — a reader without the named face gets the browser default."""
    missing = [
        decl.strip()
        for decl in _FONT_SHORTHAND.findall(gb.HTML_TEMPLATE)
        if not any(g in decl for g in _GENERIC_FAMILIES)
    ]
    assert not missing, f"font shorthand with no generic family: {missing}"


def test_template_no_css_rule_declares_the_same_property_twice():
    """A duplicated declaration means one of them is dead code."""
    offenders = []
    for body in re.findall(r"\{([^{}]*)\}", gb.HTML_TEMPLATE):
        props = [
            d.split(":", 1)[0].strip().lower()
            for d in body.split(";")
            if ":" in d and not d.strip().startswith("--")
        ]
        dupes = {p for p in props if props.count(p) > 1}
        if dupes:
            offenders.append((sorted(dupes), body.strip()[:60]))
    assert not offenders, f"duplicate CSS declarations: {offenders}"


def _external_script_tags():
    """Every <script src=…> pointing off-origin.

    Selected by SCHEME, not by whether the host happens to contain "cdn." — an
    unpkg.com or a raw.githack.com URL is exactly as remote, and a filter that
    skipped it would silently exempt the next one somebody adds.
    """
    tags = re.findall(r"<script[^>]*\bsrc=[^>]*?>", gb.HTML_TEMPLATE)
    return [t for t in tags if re.search(r'\bsrc="(?:https?:)?//', t)]


def test_template_every_external_script_carries_subresource_integrity():
    """Web:S5725, and rules/web/security.md: SRI for CDN-loaded scripts."""
    tags = _external_script_tags()
    assert tags, "no external script found — the selector, not the page, is wrong"
    for tag in tags:
        assert re.search(r'\bintegrity="sha(?:256|384|512)-\S+"', tag), (
            f"external script without a real integrity hash: {tag[:110]}"
        )
        # The VALUE matters: SRI is only enforced on a cross-origin fetch made
        # in anonymous (or credentialed) mode. A bare `crossorigin` attribute
        # is not the same guarantee, so assert what the spec requires.
        assert re.search(r'\bcrossorigin="(?:anonymous|use-credentials)"', tag), (
            f"integrity without a valid crossorigin value: {tag[:110]}"
        )


def test_template_external_script_version_is_pinned_exactly():
    """A floating major CANNOT carry an integrity hash — the bytes may change.

    Pinning is what makes the hash meaningful rather than a time bomb, so this
    and the integrity test belong together.

    FAILS CLOSED on an unrecognised pinning scheme. The earlier version skipped
    any src without "cdn." in it and required one exact `@X.Y.Z/` shape, so a
    host it did not recognise was exempted silently. Here an unknown scheme is
    a FAILURE that names itself, because "I could not tell whether this is
    pinned" is not "this is pinned".
    """
    known_pins = (
        r"@\d+\.\d+\.\d+/",  # npm-style: /npm/pkg@1.2.3/file.js
        r"[?&]v(?:er|ersion)?=\d+\.\d+\.\d+\b",  # query-string pinning
        r"/\d+\.\d+\.\d+/",  # path-segment pinning: /ajax/libs/x/1.2.3/x.js
    )
    for tag in _external_script_tags():
        src = re.search(r'\bsrc="([^"]+)"', tag).group(1)
        assert any(re.search(p, src) for p in known_pins), (
            f"external script is not pinned to an exact version, or uses a "
            f"pinning scheme this test does not recognise: {src}"
        )


def test_template_uses_globalthis_rather_than_window():
    assert "window." not in gb.HTML_TEMPLATE
    assert "globalThis.mermaid" in gb.HTML_TEMPLATE
