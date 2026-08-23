"""diagrams.py — the four explanatory diagrams, one function each."""

import os

from .chrome import (
    AMBER,
    DIM,
    EDGE,
    TEAL,
    TEAL_DIM,
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


def draw_arch():
    W, H = 1600, 900
    img, d = canvas(W, H)
    d.text((60, 44), "ONE CONSCIENCE, MANY BODIES", font=mono(34, bold=True), fill=TEXT)
    label(
        d, 60, 92, "every tool call, every harness -> one law engine -> one ledger", 22
    )

    harnesses = [
        ("any agent", "extension"),
        ("any agent", "hook"),
        ("any agent", "rpc"),
        ("your agent", "here"),
    ]
    hy = 180
    HH = 104
    for i, (hname, hsub) in enumerate(harnesses):
        y = hy + i * (HH + 30)
        panel(d, 60, y, 330, HH, None)
        label(d, 84, y + 26, hname, 26, TEXT, bold=True)
        label(
            d,
            84,
            y + 64,
            f"attaches via {hsub}" if hsub != "here" else "one file away",
            16,
        )
        arrow(d, 390, y + HH // 2, 560, y + HH // 2)

    # nerve column
    d.rounded_rectangle(
        [560, 170, 660, hy + 4 * (HH + 30) - 30],
        radius=8,
        fill=(16, 20, 27),
        outline=TEAL_DIM,
        width=2,
    )
    for i in range(4):
        y = hy + i * (HH + 30) + HH // 2
        d.line([560, y, 610, y], fill=TEAL_DIM, width=2)
        d.line([610, y, 610, 460], fill=TEAL_DIM, width=2)
    label(d, 568, 196, "the nerve", 19, TEAL, bold=True)
    label(d, 568, 222, "~160-ln", 15)
    label(d, 568, 240, "adapter", 15)
    arrow(d, 660, 460, 800, 460)

    # the brain
    bx, by, bw, bh = 800, 330, 360, 260
    panel(d, bx, by, bw, bh, None, accent=TEAL)
    label(d, bx + 24, by + 34, "caddis-warden", 30, TEXT, bold=True)
    label(d, bx + 24, by + 78, "stateless law engine", 20, TEAL)
    for i, s in enumerate(
        ["zero dependencies", "real shell grammar", "deny / steer / allow"]
    ):
        label(d, bx + 24, by + 116 + i * 30, s, 19)
    arrow(d, bx + bw // 2, by + bh, bx + bw // 2, by + bh + 70)

    # the ledger
    lx, ly, lw, lh = bx + bw // 2 - 260, by + bh + 70, 520, 150
    panel(d, lx, ly, lw, lh, None)
    label(d, lx + 20, ly + 16, "~/.caddis/warden-ledger.jsonl", 19, TEAL)
    rows = [
        ('"from":"agent@laptop","body":"deny|git push --force origin main"', AMBER),
        ('"from":"agent@ci","body":"allow|cargo test"', DIM),
        ('"from":"reviewer@bot","body":"allow|echo ok"', DIM),
    ]
    for i, (s, c) in enumerate(rows):
        label(d, lx + 20, ly + 50 + i * 30, s[:66], 17, c, bold=(c == AMBER))
    label(
        d,
        60,
        H - 64,
        "the binary is spawned per call; the ledger on disk is the only state",
        19,
    )
    img.save(os.path.join(ASSETS, "diagram-architecture.png"))


# ── diagram 2: the verdict flow ──────────────────────────────────────────────


def draw_flow():
    W, H = 1600, 900
    img, d = canvas(W, H)
    d.text((60, 44), "THE VERDICT FLOW", font=mono(34, bold=True), fill=TEXT)
    label(
        d,
        60,
        92,
        "a tool call arrives as one length-prefixed frame; one JSON verdict leaves",
        22,
    )

    # input
    panel(d, 60, 180, 430, 190)
    label(d, 84, 200, "tool_call", 24, TEXT, bold=True)
    for i, s in enumerate(
        ["tool 4", "bash", "command 28", "git push --force origin main"]
    ):
        label(d, 84, 240 + i * 28, s, 18, TEAL if i < 3 else AMBER)

    arrow(d, 490, 275, 640, 275)

    # judge — small case motif
    jx, jy = 640, 150
    panel(d, jx, jy, 380, 250, None, accent=AMBER)
    jc = (jx + 190, jy + 128)
    shard_ring(d, jc[0], jc[1], r_out=86, r_in=56, n=13)
    glow(d, jc[0], jc[1], 7, AMBER, layers=10, spread=2)
    d.ellipse([jc[0] - 9, jc[1] - 9, jc[0] + 9, jc[1] + 9], fill=AMBER)
    label(d, jx + 16, jy + 196, "the law engine judges", 19, TEXT)

    # three outcomes
    outs = [
        ("ALLOW", "the call runs", '"verdict":"allow"', DIM),
        ("STEER", "runs; law rides the result", '"verdict":"steer","law":"..."', TEAL),
        ("DENY", "blocked before the shell", '"verdict":"deny","reason":"..."', AMBER),
    ]
    oy = 480
    for i, (name, gloss, js, col) in enumerate(outs):
        y = oy + i * 118
        arrow(d, jx + 190, jy + 250, 260, y + 40)
        panel(d, 260, y, 620, 96, None)
        label(d, 284, y + 16, name, 26, col, bold=True)
        label(d, 284, y + 52, gloss, 19)
        label(d, 940, y + 34, js, 19, col)

    # ledger strip
    panel(d, 1100, 150, 440, 620)
    label(d, 1124, 170, "LEDGER (every verdict)", 20, TEXT, bold=True)
    sample = [
        ("seq 1030", "deny", AMBER),
        ("seq 1031", "steer", TEAL),
        ("seq 1032", "allow", DIM),
        ("seq 1033", "allow", DIM),
    ]
    for i, (sq, vd, col) in enumerate(sample):
        y = 226 + i * 50
        d.line([1124, y + 38, 1516, y + 38], fill=EDGE, width=1)
        label(d, 1124, y, sq, 18, DIM)
        label(d, 1400, y, vd, 18, col, bold=True)
    label(d, 1124, 470, '"what did the agent do', 18, DIM)
    label(d, 1124, 496, ' last night?" - one grep', 18, DIM)
    arrow(d, 950, 700, 1100, 660)
    label(
        d,
        60,
        H - 64,
        "unreadable verdict -> BLOCK (judgement fails closed)   binary missing -> allow, loudly",
        19,
    )
    img.save(os.path.join(ASSETS, "diagram-verdict.png"))


# ── diagram 3: onboarding self-proof ─────────────────────────────────────────


def draw_onboard():
    W, H = 1600, 700
    img, d = canvas(W, H)
    d.text((60, 44), "ONBOARDING IS THE PROOF", font=mono(34, bold=True), fill=TEXT)
    label(d, 60, 92, "an install that cannot show you a denial is not an install", 22)

    steps = [
        ("1  BUILD", "cargo build --release", "one binary, ~2 s", TEAL),
        ("2  WIRE", "one adapter file per harness", "CALLER stamped -> from:", TEAL),
        ("3  PROVE", "force-push frame in", "DENY + allow + ledger row", AMBER),
    ]
    x = 60
    for i, (t, a, b, col) in enumerate(steps):
        panel(d, x, 190, 430, 260, None, accent=col)
        label(d, x + 24, 214, t, 28, col, bold=True)
        label(d, x + 24, 262, a, 20, TEXT)
        label(d, x + 24, 296, b, 19)
        if i < 2:
            arrow(d, x + 430, 320, x + 500, 320)
        x += 500

    # the proof readout
    panel(d, 60, 490, 1480, 150)
    lines = [
        ("$ onboard", DIM),
        ("onboard: binary installed at ~/.caddis/bin/caddis-warden", TEXT),
        ("onboard: PROOF - force-push DENIED (ledger seq 1120), echo ALLOWED.", AMBER),
    ]
    for i, (s, c) in enumerate(lines):
        label(d, 84, 512 + i * 34, s, 20, c, bold=(c == AMBER))
    img.save(os.path.join(ASSETS, "diagram-onboard.png"))


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
    label(d, 84, 190, "~/.caddis/warden-ledger.jsonl  (append-only)", 20, TEAL)
    rows = [
        ("seq 41", 'from":"agent@laptop"', "deny|git push --force origin main", AMBER),
        ("seq 42", 'from":"agent@laptop"', "allow|cargo test -p core", DIM),
        ("seq 43", 'from":"agent@ci"', "allow|cargo build --release", DIM),
        ("seq 44", 'from":"reviewer@bot"', "steer|git commit -am ...", TEAL),
        ("seq 45", 'from":"agent@laptop"', "deny|curl ... | sh", AMBER),
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
