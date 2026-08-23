"""md-fix.py — bring the markdown to canonical shape (CI superstrict, no config).

Structural fixes (blank lines around fences/headings, fence languages, table
pipe spacing, plain images) + paragraph rewrap to 80 cols. Run repeatedly
until markdownlint-cli2 is silent; hand-fix whatever remains.
"""
import re
import textwrap

FILES = [
    "README.md", "PROTOCOL.md", "LAWS.md", "ONBOARD.md", "CHANGELOG.md",
    "adapters/README.md", "assets/DESIGN-PHILOSOPHY.md",
    "DEFECT-ledger-truncates-at-first-newline.md",
]


def fix_structural(text):
    out = []
    in_code = False
    for ln in text.split("\n"):
        stripped = ln.strip()
        if stripped.startswith("```"):
            if not in_code:
                if out and out[-1].strip() != "":
                    out.append("")          # MD031: blank line before opening
                if stripped == "```":
                    ln = "```text"          # MD040: bare fence gets a language
            in_code = not in_code
            out.append(ln)
            continue
        if in_code:
            out.append(ln)
            continue
        if stripped.startswith("#") and out and out[-1].strip() != "" \
                and not out[-1].startswith("#"):
            out.append("")                  # MD022: blank line before heading
        if stripped.startswith("|") and not re.match(r"^\|[-| :]+\|$", stripped):
            ln = re.sub(r"\|([^ |\n])", r"| \1", ln)   # MD060 pipe spacing
            ln = re.sub(r"([^ |])\|", r"\1 |", ln)
        out.append(ln)
    text = "\n".join(out)
    text = re.sub(r"```\n(?!\n|```|$)", "```\n\n", text)  # blank after close
    text = re.sub(
        r'<p><img src="([^"]+)"[^>]*alt="([^"]+)"></p>', r'![\2](\1)', text)
    text = re.sub(r'<p><img src="([^"]+)"[^>]*></p>', r'![](\1)', text)
    return text


def wrap_paragraphs(text):
    out_lines, para = [], []

    def flush():
        if not para:
            return
        first = para[0]
        lstripped = first.lstrip()
        indent = first[:len(first) - len(lstripped)]
        if lstripped.startswith(("#", "|", ">")):
            out_lines.extend(para)          # headings/quotes: leave as-is
        elif re.match(r"^[-*+] ", lstripped):
            m = re.match(r"^([-*+]) ", lstripped)
            bullet = (m.group(1) if m else "-") + " "
            body = " ".join([lstripped[len(bullet):]] +
                            [x.strip() for x in para[1:]])
            out_lines.extend(textwrap.wrap(
                body, width=80, initial_indent=indent + bullet,
                subsequent_indent=indent + "  ",
                break_long_words=False, break_on_hyphens=False))
        elif re.match(r"^\d+\. ", lstripped):
            m = re.match(r"^(\d+)\. ", lstripped)
            bullet = m.group(0) if m else "1. "
            body = " ".join([lstripped[len(bullet):]] +
                            [x.strip() for x in para[1:]])
            out_lines.extend(textwrap.wrap(
                body, width=80, initial_indent=indent + bullet,
                subsequent_indent=indent + " " * len(bullet),
                break_long_words=False, break_on_hyphens=False))
        else:
            body = " ".join(x.strip() for x in para)
            out_lines.extend(textwrap.wrap(
                body, width=80, break_long_words=False, break_on_hyphens=False))
        para.clear()

    in_code = False
    for ln in text.split("\n"):
        if ln.lstrip().startswith("```"):
            flush()
            in_code = not in_code
            out_lines.append(ln)
            continue
        if in_code or ln.strip().startswith("|"):
            flush()
            out_lines.append(ln)
            continue
        if ln.strip() == "":
            flush()
            out_lines.append("")
            continue
        para.append(ln)
    flush()
    return "\n".join(out_lines)


for f in FILES:
    text = open(f, encoding="utf-8").read()
    text = fix_structural(text)
    text = wrap_paragraphs(text)
    text = re.sub(r"\n{3,}", "\n\n", text)
    open(f, "w", encoding="utf-8", newline="\n").write(text)
    print("processed", f)
