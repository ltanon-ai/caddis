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
    out = gb._render_fence("python", ["x = 1 < 2"])
    assert '<pre class="code"><code>' in out
    assert "1 &lt; 2" in out


def test_git_takes_a_list_of_arguments_not_a_string():
    """The annotation said `str` while every caller passed a list.

    On a str, `["git", "-C", WS] + args` raises rather than building a
    character-wise argv — so this asserts the contract the callers rely on.
    """
    assert gb.git(["rev-parse", "--is-inside-work-tree"]) in ("true", "false", "?")


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


def test_template_every_cdn_script_carries_subresource_integrity():
    """Web:S5725, and rules/web/security.md: SRI for CDN-loaded scripts."""
    for tag in re.findall(r"<script[^>]*\bsrc=[^>]*>", gb.HTML_TEMPLATE):
        assert "integrity=" in tag, f"CDN script without integrity: {tag[:90]}"
        assert "crossorigin=" in tag, f"integrity without crossorigin: {tag[:90]}"


def test_template_cdn_version_is_pinned_exactly():
    """A floating major CANNOT carry an integrity hash — the bytes may change.

    Pinning is what makes the hash above meaningful rather than a time bomb,
    so the two assertions belong together.
    """
    for src in re.findall(r'<script[^>]*\bsrc="([^"]+)"', gb.HTML_TEMPLATE):
        if "cdn." not in src:
            continue
        assert re.search(r"@\d+\.\d+\.\d+/", src), f"unpinned CDN version: {src}"


def test_template_uses_globalthis_rather_than_window():
    assert "window." not in gb.HTML_TEMPLATE
    assert "globalThis.mermaid" in gb.HTML_TEMPLATE
