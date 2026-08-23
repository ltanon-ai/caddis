"""diagrams.py — the system diagrams (architecture, verdict flow,
onboarding), one function each."""

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

    ANY_AGENT = "any agent"
    harnesses = [
        (ANY_AGENT, "extension"),
        (ANY_AGENT, "hook"),
        (ANY_AGENT, "rpc"),
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


# The record-keeping diagrams (ledger, memory, cards) live in
# diagrams_records.py — split under the 300-line hard cap, 2026-08-23.

