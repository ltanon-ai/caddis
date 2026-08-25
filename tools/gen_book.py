#!/usr/bin/env python3
"""CADDIS BOOK generator — agent book with DNA styling.

Freshness guarantee (the mechanism):
  1. Every chapter embeds its source file's mtime + git blob hash.
  2. BOOK.html header embeds generation timestamp + HEAD commit.
  3. `--check` mode: exits 1 if any source is newer than BOOK.html
     (docs changed, book stale) — wire it into any commit procedure;
     TRUST WEDGE integration planned post-freeze (organ hook).
  4. Mermaid diagrams render client-side (CDN, graceful offline fallback
     to code blocks).

Usage:
  python tools/gen_book.py            # generate docs/BOOK.html
  python tools/gen_book.py --check    # freshness check (exit 1 = stale)
"""
import os, sys, subprocess, hashlib, datetime, html, re

WS = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BOOK = os.path.join(WS, "docs", "BOOK.html")

PRIORITY = ["README.md", "docs/STATE.md", "docs/DECISIONS.md", "docs/ARCHITECTURE.md",
            "docs/HANDOFF.md", "docs/MASTER-PROMPT.md", "docs/API.md", "docs/TELEMETRY.md",
            "docs/54-rework-verdict.md", "docs/41-riverbed-context.md", "docs/42-pools-dna.md",
            "docs/43-clear-water.md", "docs/43-clear-water-vision.md", "docs/55-forge-gate-design.md",
            "docs/26-testing-doctrine.md", "docs/19-gap-sweep.md", "docs/11-three-radicals.md",
            "docs/12-llm-callsites.md", "AGENTS.md", "docs/TESTING-GUIDE.md"]

def git(short: str) -> str:
    r = subprocess.run(["git", "-C", WS] + short, capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 else "?"

def blob_hash(path: str) -> str:
    try:
        return git(["hash-object", path])[:10]
    except Exception:
        return "?"

def collect():
    files = []
    for p in PRIORITY:
        fp = os.path.join(WS, p.replace("/", os.sep))
        if os.path.exists(fp):
            files.append((p, fp))
    for f in sorted(os.listdir(os.path.join(WS, "docs"))):
        if f.endswith(".md"):
            rel = f"docs/{f}"
            if rel not in [p for p, _ in files]:
                files.append((rel, os.path.join(WS, "docs", f)))
    return files

def esc(s): return html.escape(s, quote=False)

def chapter(idx, rel, fp):
    body = open(fp, encoding="utf-8", errors="replace").read()
    mtime = datetime.datetime.fromtimestamp(os.path.getmtime(fp)).strftime("%Y-%m-%d %H:%M")
    bh = blob_hash(rel)
    # markdown-lite render: headers, code fences, mermaid passthrough
    parts = [f'<section class="ch" id="ch{idx}">']
    parts.append(f'<div class="chmeta"><span class="path">{esc(rel)}</span>'
                 f'<span class="stamp">mtime {mtime} · blob {bh}</span></div>')
    in_code, lang, buf = False, "", []
    for line in body.splitlines():
        if not in_code and line.startswith("```"):
            in_code, lang = True, line[3:].strip(); buf = []
            continue
        if in_code and line.strip() == "```":
            in_code = False
            content = esc("\n".join(buf))
            if lang == "mermaid":
                parts.append(f'<div class="mmwrap"><pre class="mermaid">{content}</pre></div>')
            else:
                parts.append(f'<pre class="code"><code>{content}</code></pre>')
            continue
        if in_code: buf.append(line); continue
        if line.startswith("# "): parts.append(f'<h2>{esc(line[2:])}</h2>')
        elif line.startswith("## "): parts.append(f'<h3>{esc(line[3:])}</h3>')
        elif line.startswith("### "): parts.append(f'<h4>{esc(line[4:])}</h4>')
        elif re.match(r"^\s*[-*] ", line):
            txt = line.lstrip()[2:]
            parts.append('<div class="li">' + esc(txt) + '</div>')
        elif line.startswith("|"): parts.append(f'<div class="trow">{esc(line)}</div>')
        elif line.strip(): parts.append(f'<p>{esc(line)}</p>')
        else: parts.append("<div class='sp'></div>")
    parts.append("</section>")
    return "".join(parts)

def main():
    check_only = "--check" in sys.argv
    files = collect()
    if check_only:
        if not os.path.exists(BOOK):
            print("STALE: BOOK.html does not exist"); sys.exit(1)
        book_t = os.path.getmtime(BOOK)
        stale = [rel for rel, fp in files if os.path.getmtime(fp) > book_t]
        if stale:
            print("STALE chapters:", ", ".join(stale[:10])); sys.exit(1)
        print("FRESH: book matches sources"); sys.exit(0)
    gen_ts = datetime.datetime.now().strftime("%Y-%m-%d %H:%M")
    head = git(["rev-parse", "--short", "HEAD"])
    toc = "".join(f'<a href="#ch{i}">{esc(rel)}</a>' for i, (rel, _) in enumerate(files))
    body = "".join(chapter(i, rel, fp) for i, (rel, fp) in enumerate(files))
    html_doc = HTML_TEMPLATE.replace("__TOC__", toc).replace("__BODY__", body)\
        .replace("__TS__", gen_ts).replace("__HEAD__", head).replace("__N__", str(len(files)))
    os.makedirs(os.path.dirname(BOOK), exist_ok=True)
    open(BOOK, "w", encoding="utf-8", newline="\n").write(html_doc)
    print(f"BOOK.html: {len(files)} chapters, {len(html_doc)//1024} KB, HEAD {head} @ {gen_ts}")

HTML_TEMPLATE = """<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8">
<title>CADDIS — The Agent Book</title>
<script src="https://cdn.jsdelivr.net/npm/mermaid@10/dist/mermaid.min.js"></script>
<style>
:root{
  --gold:#d7ba7a; --gold-b:#e8cd8f; --ink:#e8e6e3; --dim:#6e6e78;
  --bg:#101014; --panel:#18181e; --line:#2a2a33; --teal:#7fbfb0;
  --green:#8bc34a; --red:#e07070;
}
body{background:var(--bg);color:var(--ink);font:15px/1.6 Georgia,'Times New Roman',serif;margin:0;display:flex}
#side{width:300px;min-width:300px;height:100vh;overflow-y:auto;background:var(--panel);border-right:1px solid var(--line);padding:18px 0;position:sticky;top:0}
#side h1{font:700 20px/1.2 Georgia;color:var(--gold);padding:8px 20px 2px}
#side .sub{color:var(--dim);font-size:12px;padding:0 20px 14px;border-bottom:1px solid var(--line)}
#side a{display:block;color:var(--ink);text-decoration:none;font:12.5px/1.5 Consolas,monospace;padding:4px 20px}
#side a:hover{background:#22222c;color:var(--gold-b)}
#fresh{position:fixed;top:0;right:0;background:var(--gold);color:#101014;font:700 11px monospace;padding:3px 10px;border-radius:0 0 0 8px;z-index:9}
#main{flex:1;padding:30px 48px;max-width:960px}
.ch{border:1px solid var(--line);border-radius:10px;padding:22px 28px;margin:26px 0;background:#131318}
.chmeta{display:flex;justify-content:space-between;font:11px Consolas,monospace;color:var(--dim);border-bottom:1px solid var(--line);padding-bottom:8px;margin-bottom:14px}
.chmeta .path{color:var(--teal)}
h2{color:var(--gold);font:700 24px/1.25 Georgia;margin:18px 0 8px;border-bottom:1px solid var(--line);padding-bottom:6px}
h3{color:var(--gold-b);font:700 18px Georgia;margin:16px 0 6px}
h4{color:var(--teal);font:700 15px Georgia;margin:12px 0 4px}
p{margin:7px 0}
.li{padding-left:18px;margin:4px 0}
.li:before{content:"\\25CF\\00a0";color:var(--gold);font-size:9px;vertical-align:2px}
.trow{font:12px Consolas,monospace;color:var(--dim);white-space:pre-wrap}
.code{background:#0b0b0f;border:1px solid var(--line);border-left:3px solid var(--teal);border-radius:6px;padding:12px 14px;overflow-x:auto;font:12.5px/1.5 Consolas,monospace;color:#c9c7c2;margin:10px 0;white-space:pre-wrap}
.mmwrap{background:#0d0d12;border:1px solid var(--gold);border-radius:8px;padding:14px;margin:14px 0;display:flex;justify-content:center}
.mmwrap:before{content:"mermaid";display:block;font:10px monospace;color:var(--gold);margin:-14px 0 8px -14px;padding:2px 8px;background:var(--gold);color:#101014;border-radius:6px 0 6px 0;position:relative;top:0}
.mermaid{background:transparent!important;font-family:Consolas,monospace}
.sp{height:8px}
</style></head><body>
<div id="fresh">GENERATED __TS__ · HEAD __HEAD__ · __N__ chapters</div>
<nav id="side"><h1>&#129417; CADDIS</h1><div class="sub">The Agent Book<br>living documentation</div>__TOC__</nav>
<main id="main">__BODY__</main>
<script>if(window.mermaid){mermaid.initialize({startOnLoad:true,theme:'dark',themeVariables:{primaryColor:'#2a2a33',primaryTextColor:'#e8e6e3',primaryBorderColor:'#d7ba7a',lineColor:'#7fbfb0',fontSize:'13px'}});}else{document.querySelectorAll('.mermaid').forEach(e=>e.style.color='#6e6e78');}</script>
</body></html>"""

if __name__ == "__main__":
    main()
