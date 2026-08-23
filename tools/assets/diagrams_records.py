"""diagrams_records.py — the record-keeping diagrams, one function each.

Split from diagrams.py under the 300-line hard cap (quality:code-metrics,
measured 2026-08-23): this module owns the append-only story — ledger,
memory, and the card lifecycle. The system diagrams (architecture, verdict
flow, onboarding) stay in diagrams.py.
"""

import os

from .chrome import (
    AMBER,
    DIM,
    EDGE,
    TEAL,
    TEXT,
    arrow,
    canvas,
    label,
    mono,
    panel,
)

ASSETS = os.path.join(
    os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
    "assets",
)


def draw_ledger():
    """How the ledger works: one append-only row per verdict, whoever called."""
    W, H = 1600, 900
    img, d = canvas(W, H)
    d.text((60, 44), "HOW THE LEDGER WORKS", font=mono(34, bold=True), fill=TEXT)
    label(
        d,
        60,
        92,
        "allow, steer and deny alike - every decision appends one row, nothing is ever edited",
        22,
    )

    # the file, growing downward
    panel(d, 60, 170, 620, 600)
    LAPTOP = 'from":"agent@laptop"'
    rows = [
        ("seq 41", LAPTOP, "deny|git push --force origin main", AMBER),
        ("seq 42", LAPTOP, "allow|cargo test -p core", DIM),
        ("seq 43", 'from":"agent@ci"', "allow|cargo build --release", DIM),
        ("seq 44", 'from":"reviewer@bot"', "steer|git commit -am ...", TEAL),
        ("seq 45", LAPTOP, "deny|curl ... | sh", AMBER),
    ]
    for i, (sq, frm, body, col) in enumerate(rows):
        y = 240 + i * 96
        d.rounded_rectangle(
            [84, y, 656, y + 78], radius=6, fill=(24, 30, 39), outline=EDGE, width=1
        )
        label(d, 100, y + 8, sq, 20, col, bold=True)
        label(d, 200, y + 8, frm, 18, DIM)
        label(d, 100, y + 40, body[:52], 17, TEXT)
    label(d, 84, 736, "newest row at the bottom - the file only ever grows", 17, DIM)

    # anatomy of one row
    panel(d, 740, 170, 800, 250)
    label(d, 764, 190, "ANATOMY OF ONE ROW", 20, TEXT, bold=True)
    parts = [
        ("seq", "monotonic row number - order is proof of order"),
        ("from", "WHICH caller made the call - every agent is attributable"),
        ("type", "tool.bash / tool.write / ... what kind of call"),
        ("body", "verdict | the command's first line | path"),
        ("ts", "unix seconds - when the judgement happened"),
    ]
    for i, (k, v) in enumerate(parts):
        label(d, 764, 226 + i * 36, k, 19, TEAL, bold=True)
        label(d, 900, 226 + i * 36, v, 18)

    # the audit question
    panel(d, 740, 460, 800, 310)
    label(d, 764, 480, "WHAT IT ANSWERS", 20, TEXT, bold=True)
    for i, q in enumerate(
        [
            '"what did the agent do last night?"  - one grep',
            '"which of my agents was denied what?" - filter deny, group by from',
            '"was the guard even running?"         - gaps in seq say so, loudly',
        ]
    ):
        label(d, 764, 520 + i * 40, q, 18)
    label(d, 764, 664, "a warden that records only its refusals cannot answer", 17, DIM)
    label(d, 764, 690, "the nightly question - so everything is recorded", 17, DIM)
    img.save(os.path.join(ASSETS, "diagram-ledger.png"))


def draw_memory():
    """How caddis remembers: one attributed row per decision, forever."""
    W, H = 1600, 900
    img, d = canvas(W, H)
    d.text((60, 44), "HOW CADDIS REMEMBERS", font=mono(34, bold=True), fill=TEXT)
    label(d, 60, 92, "one decision -> one row -> append-only, attributed, never rewritten by the engine", 22)

    panel(d, 60, 170, 700, 600)
    label(d, 84, 190, "~/.caddis/warden-ledger.jsonl", 20, TEAL)
    rows = [
        ("seq 201", "from: agent@ci", "allow|cargo build", DIM),
        ("seq 202", "from: agent@laptop", "deny|git push --force origin main", AMBER),
        ("seq 203", "from: agent@laptop", "steer|docker run --rm api", TEAL),
        ("seq 204", "from: reviewer@bot", "allow|cargo test", DIM),
        ("seq 205", "from: agent@ci", "deny|curl ... | sh", AMBER),
    ]
    for i, (sq, frm, body, col) in enumerate(rows):
        y = 240 + i * 96
        d.rounded_rectangle([84, y, 736, y + 78], radius=6, fill=(24, 30, 39), outline=EDGE)
        label(d, 100, y + 8, sq, 20, col, bold=True)
        label(d, 230, y + 8, frm, 17, DIM)
        label(d, 100, y + 40, body[:44], 17, TEXT)
    label(d, 84, 736, "the file only grows - newest row at the bottom", 17, DIM)

    panel(d, 820, 170, 720, 280)
    label(d, 844, 190, "READING THE MEMORY", 20, TEXT, bold=True)
    for i, q in enumerate([
        "grep deny| + tail     -> the nightly question",
        "grep from:my-agent    -> one agent's story",
        "gaps in seq           -> the guard was not running",
    ]):
        label(d, 844, 228 + i * 40, q, 18)

    panel(d, 820, 490, 720, 280)
    label(d, 844, 510, "WHAT IT IS NOT", 20, TEXT, bold=True)
    for i, q in enumerate([
        "not tamper-proof - append-only is behavior, not protection",
        "not content memory - which file, never what was written",
        "not a model context - an audit memory for the operator",
    ]):
        label(d, 844, 548 + i * 40, q, 18, DIM)
    img.save(os.path.join(ASSETS, "diagram-memory.png"))


def draw_cards():
    """The card lifecycle: falsifiable completion, mechanical proof."""
    W, H = 1600, 700
    img, d = canvas(W, H)
    d.text((60, 44), "THE CARD LIFECYCLE", font=mono(34, bold=True), fill=TEXT)
    label(d, 60, 92, "a work unit whose DONE is checkable and whose proof runs - claims are noise, evidence is the artifact", 22)

    steps = [
        ("WRITE", "card.md", "Done-When + RED-TEST", TEAL),
        ("VALIDATE", "schema check", "caddis-card validate", TEAL),
        ("WORK", "RED first", "proof fails for the stated reason", TEAL),
        ("PROVE", "gate decides", "RED-TEST passes, Done-When holds", AMBER),
    ]
    x = 60
    for i, (t, a, b, col) in enumerate(steps):
        panel(d, x, 190, 330, 240, None, accent=col)
        label(d, x + 24, 214, f"{i + 1}  {t}", 28, col, bold=True)
        label(d, x + 24, 262, a, 20, TEXT)
        label(d, x + 24, 296, b[:34], 18)
        if i < 3:
            arrow(d, x + 330, 310, x + 400, 310)
        x += 400

    panel(d, 60, 480, 1480, 160)
    label(d, 84, 502, "if the proof fails for a DIFFERENT reason - the card was wrong:", 20, TEXT)
    label(d, 84, 536, "fix the card first; a rejected card is context: its lesson rides the next card", 19, DIM)
    label(d, 84, 570, "every caddis change arrives as a card whose RED-TEST ran", 19, TEAL)
    img.save(os.path.join(ASSETS, "diagram-cards.png"))
