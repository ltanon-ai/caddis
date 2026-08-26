"""diagrams_evidence.py — the evidence-program diagrams.

The 0.3.0 readers made the ledger legible: the loop (warden -> ledger ->
five readers) and the card validator's two oracles (v1 spine, strict
contract, plan decomposition). The records module owns the append-only
mechanics; this one owns what reads them and how a card is checked.
"""

import os

from .chrome import (
    AMBER,
    DIM,
    TEAL,
    TEXT,
    arrow,
    canvas,
    glow,
    label,
    mono,
    panel,
    shard_ring,
)

ASSETS = os.path.join(
    os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
    "assets",
)


def draw_loop():
    """Memory into evidence: the warden, the ledger, the five readers."""
    W, H = 1600, 900
    img, d = canvas(W, H)
    d.text((60, 44), "FROM MEMORY TO EVIDENCE", font=mono(34, bold=True), fill=TEXT)
    label(d, 60, 92, "the loop: judge -> record -> prove -> retire the rules that stopped earning", 22)

    # the warden — the judge, so the amber accent and the spark
    panel(d, 560, 150, 480, 210, None, accent=AMBER)
    shard_ring(d, 668, 255, r_out=86, r_in=56, n=13)
    glow(d, 668, 255, 7, AMBER, layers=10, spread=2)
    d.ellipse([659, 246, 677, 264], fill=AMBER)
    label(d, 784, 186, "the warden", 26, TEXT, bold=True)
    label(d, 784, 228, "every tool call", 19)
    label(d, 784, 262, "allow", 19, DIM, bold=True)
    label(d, 872, 262, "steer", 19, TEAL, bold=True)
    label(d, 960, 262, "deny", 19, AMBER, bold=True)
    label(d, 784, 300, "before the shell runs", 16)

    arrow(d, 800, 360, 800, 418)
    label(d, 816, 380, "one attributed row, always", 17, TEAL)

    # the ledger
    panel(d, 560, 418, 480, 152)
    label(d, 584, 436, "the ledger - append-only JSONL, local, greppable", 19, TEAL)
    rows = [
        ("seq 41  deny  |git push --force origin main", AMBER),
        ("seq 42  allow |cargo test -p core", DIM),
        ("seq 43  steer |docker run --rm api", TEAL),
    ]
    for i, (s, c) in enumerate(rows):
        label(d, 584, 472 + i * 28, s, 17, c, bold=(c == AMBER))

    # the bus fan-out: one drop, one rail, five taps
    readers = [
        ("receipt", TEAL, ["what one agent", "actually did -", "cited by row"]),
        ("card", TEAL, ["a work unit", "becomes a fact", "in the ledger"]),
        ("laws", TEAL, ["which rules", "earn their", "place"]),
        ("propose-laws", TEAL, ["the missing rule,", "cost attached,", "before it binds"]),
        ("attest", AMBER, ["proof anyone can", "re-check", "(--verify)"]),
    ]
    centers = [60 + i * 304 + 142 for i in range(5)]
    arrow(d, 800, 570, 800, 606)
    d.line([centers[0], 610, centers[-1], 610], fill=(66, 100, 103), width=2)
    for cx in centers:
        arrow(d, cx, 610, cx, 650)

    for i, (name, accent, gloss) in enumerate(readers):
        x = 60 + i * 304
        panel(d, x, 650, 284, 180, None, accent=accent)
        label(d, x + 20, 672, name, 24, accent, bold=True)
        for j, line in enumerate(gloss):
            label(d, x + 20, 716 + j * 26, line, 17)

    label(d, 60, 856, "a ledger nothing reads is a diary - these five commands are the readers", 19, DIM)
    img.save(os.path.join(ASSETS, "diagram-loop.png"))


def draw_plan_cards():
    """One schema, two oracles: the spine, the contract, the decomposition."""
    W, H = 1600, 700
    img, d = canvas(W, H)
    d.text((60, 44), "ONE SCHEMA, TWO ORACLES", font=mono(34, bold=True), fill=TEXT)
    label(d, 60, 92, "a work card executes; a plan decomposes - neither passes the other's gate", 22)

    steps = [
        (60, 450, "1  THE SPINE", "validate card.md", TEAL,
         ["frontmatter", "Done-When", "RED-TEST"],
         "every card carries these"),
        (560, 450, "2  THE CONTRACT", "validate card.md --strict", TEAL,
         ["+ level, blast,", "+ claims-forbidden,", "+ anchors, allowlist"],
         "nothing renegotiable at run time"),
        (1060, 480, "3  THE DECOMPOSITION", "validate plan.md --plan", AMBER,
         ["CHILDREN - id, order,", "          paths, symbols",
          "REVIEW - reviewer, verdict, checks"],
         "decomposition, not execution"),
    ]
    for x, w, t, cmd, accent, body, gloss in steps:
        panel(d, x, 170, w, 250, None, accent=accent)
        label(d, x + 24, 194, t, 28, accent, bold=True)
        label(d, x + 24, 244, cmd, 20, TEXT)
        for i, line in enumerate(body):
            label(d, x + 24, 284 + i * 26, line, 18)
        label(d, x + 24, 378, gloss, 17, DIM)

    arrow(d, 510, 295, 560, 295)
    arrow(d, 1010, 295, 1060, 295)

    panel(d, 60, 460, 1480, 180)
    cmds = [
        "$ cargo run -p caddis-card --example validate -- card.md",
        "$ cargo run -p caddis-card --example validate -- card.md --strict",
        "$ cargo run -p caddis-card --example validate -- plan.md --plan",
    ]
    for i, s in enumerate(cmds):
        label(d, 84, 484 + i * 32, s, 18, DIM)
    label(d, 84, 590, "a plan never passes --strict; a work card never needs --plan", 20, AMBER, bold=True)
    img.save(os.path.join(ASSETS, "diagram-plans.png"))
