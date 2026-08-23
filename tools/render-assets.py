"""render-assets.py — draw the caddis visual identity (Quiet Verdict movement).

Regenerates every PNG in assets/ deterministically from code:
    python tools/render-assets.py

Palette (3 + neutrals, luminance-first): ink #0E1116, teal-steel, amber.
Amber is sacred: it marks life and judgement only.
"""
from PIL import Image, ImageDraw, ImageFont
import math, os

INK = (14, 17, 22)
PANEL = (19, 24, 32)
EDGE = (42, 50, 60)
TEAL = (111, 163, 166)
TEAL_DIM = (66, 100, 103)
AMBER = (232, 161, 61)
TEXT = (200, 207, 214)
DIM = (124, 134, 143)
FONTS = r"C:\Windows\Fonts"

def font(name, size):
    return ImageFont.truetype(os.path.join(FONTS, name), size)

def mono(size, bold=False):
    return font("consolab.ttf" if bold else "consola.ttf", size)

def ui(size, light=True):
    return font("segoeuil.ttf" if light else "segoeui.ttf", size)

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ASSETS = os.path.join(ROOT, "assets")

def canvas(w, h):
    img = Image.new("RGB", (w, h), INK)
    return img, ImageDraw.Draw(img)

def glow(d, cx, cy, r, color, layers=26, spread=3.2):
    """Soft radial glow built from concentric translucent circles."""
    overlay = Image.new("RGBA", d._image.size, (0, 0, 0, 0))
    od = ImageDraw.Draw(overlay)
    for i in range(layers, 0, -1):
        rr = r + (spread * r) * i / layers
        a = int(70 * (1 - i / layers) ** 2.2)
        od.ellipse([cx - rr, cy - rr, cx + rr, cy + rr], fill=color + (a,))
    d._image.paste(Image.alpha_composite(d._image.convert("RGBA"), overlay).convert("RGB"), (0, 0))

# ── the mark: shard lattice (the case) around one spark ─────────────────────

def shard_ring(d, cx, cy, r_out, r_in, n, seed=7):
    """n stones laid TANGENTIALLY into a ring — a caddis case, shingle by
    shingle. Radial teeth read as a sawblade no matter how much you jitter
    them (measured twice in review, v0/v1); found materials lie ALONG the
    current, not pointing out of it. Each stone varies in length, thickness,
    radial depth, tilt and overlap; the ring is drawn in order so neighbors
    shingle over each other like laid twigs."""
    state = seed
    def rnd():
        nonlocal state
        state = (state * 1103515245 + 12345) % (2**31)
        return state / 2**31
    a = -math.pi / 2
    circ = 2 * math.pi * ((r_in + r_out) / 2)
    for i in range(n):
        pitch = circ / n
        a += (pitch / ((r_in + r_out) / 2)) * (0.82 + 0.36 * rnd())
        L = pitch * (1.25 + 0.45 * rnd())          # long axis, overlapping
        W = (r_out - r_in) * (0.42 + 0.34 * rnd()) # thickness of the twig
        mid_r = (r_in + r_out) * (0.485 + 0.035 * rnd())
        t = a + math.pi / 2 + (rnd() - 0.5) * 0.35 # tangent + tilt
        ux, uy = math.cos(t), math.sin(t)          # along the twig
        vx, vy = -uy, ux                           # across it
        mx, my = cx + mid_r * math.cos(a), cy + mid_r * math.sin(a)
        hL, sk = L / 2, W * 0.35 * (rnd() - 0.5)   # slight shingle skew
        pts = [
            (mx + ux * -hL + vx * -W / 2 + uy * sk, my + uy * -hL + vy * -W / 2 - ux * sk),
            (mx + ux * hL + vx * -W / 2, my + uy * hL + vy * -W / 2),
            (mx + ux * hL + vx * W / 2, my + uy * hL + vy * W / 2),
            (mx + ux * -hL + vx * W / 2 - uy * sk, my + uy * -hL + vy * W / 2 + ux * sk),
        ]
        shade = 0.72 + 0.28 * rnd()
        col = tuple(int(c * shade) for c in TEAL)
        d.polygon(pts, fill=col)
        d.line(pts + [pts[0]], fill=INK, width=3)


def draw_logo():
    W = 1024
    img, d = canvas(W, W)
    cx, cy = W // 2, W // 2 - 30
    # hairline halo
    d.ellipse([cx - 352, cy - 352, cx + 352, cy + 352], outline=EDGE, width=2)
    d.ellipse([cx - 368, cy - 368, cx + 368, cy + 368], outline=(24, 29, 37), width=2)
    # the case
    shard_ring(d, cx, cy, r_out=318, r_in=210, n=17)
    # inner hairline ring
    d.ellipse([cx - 188, cy - 188, cx + 188, cy + 188], outline=TEAL_DIM, width=3)
    # the spark
    glow(d, cx, cy, 26, AMBER)
    d.ellipse([cx - 30, cy - 30, cx + 30, cy + 30], fill=AMBER)
    d.ellipse([cx - 44, cy - 44, cx + 44, cy + 44], outline=(150, 105, 48), width=2)
    # wordmark
    f = ui(54)
    word = "caddis"
    tw = d.textlength(word, font=f)
    d.text(((W - tw) / 2, cy + 386), word, font=f, fill=TEXT)
    f2 = mono(20)
    sub = "a conscience for coding agents"
    tw2 = d.textlength(sub, font=f2)
    d.text(((W - tw2) / 2, cy + 452), sub, font=f2, fill=DIM)
    img.save(os.path.join(ASSETS, "logo.png"))
    # favicon-scale check happens in review, not in code

# ── shared diagram chrome ────────────────────────────────────────────────────

def panel(d, x, y, w, h, title=None, accent=None):
    d.rounded_rectangle([x, y, x + w, y + h], radius=10, fill=PANEL, outline=EDGE, width=2)
    if accent:
        d.rounded_rectangle([x, y, x + w, y + 6], radius=3, fill=accent)
    if title:
        d.text((x + 18, y + 18), title, font=mono(24, bold=True), fill=TEXT)

def arrow(d, x1, y1, x2, y2, color=TEAL_DIM, w=3):
    d.line([x1, y1, x2, y2], fill=color, width=w)
    ang = math.atan2(y2 - y1, x2 - x1)
    L = 12
    d.polygon([
        (x2, y2),
        (x2 - L * math.cos(ang - 0.42), y2 - L * math.sin(ang - 0.42)),
        (x2 - L * math.cos(ang + 0.42), y2 - L * math.sin(ang + 0.42)),
    ], fill=color)

def label(d, x, y, s, size=20, color=DIM, bold=False):
    d.text((x, y), s, font=mono(size, bold=bold), fill=color)

# ── diagram 1: one conscience, many bodies ──────────────────────────────────

def draw_arch():
    W, H = 1600, 900
    img, d = canvas(W, H)
    d.text((60, 44), "ONE CONSCIENCE, MANY BODIES", font=mono(34, bold=True), fill=TEXT)
    label(d, 60, 92, "every tool call, every harness -> one law engine -> one ledger", 22)

    harnesses = ["omp", "little-coder", "prime-agent", "claude-code"]
    hy = 180
    HH = 104
    for i, hname in enumerate(harnesses):
        y = hy + i * (HH + 30)
        panel(d, 60, y, 330, HH, None)
        label(d, 84, y + 26, hname, 26, TEXT, bold=True)
        label(d, 84, y + 64, "harness", 16)
        arrow(d, 390, y + HH // 2, 560, y + HH // 2)

    # nerve column
    d.rounded_rectangle([560, 170, 660, hy + 4 * (HH + 30) - 30], radius=8,
                        fill=(16, 20, 27), outline=TEAL_DIM, width=2)
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
    for i, s in enumerate(["zero dependencies", "real shell grammar", "deny / steer / allow"]):
        label(d, bx + 24, by + 116 + i * 30, s, 19)
    arrow(d, bx + bw // 2, by + bh, bx + bw // 2, by + bh + 70)

    # the ledger
    lx, ly, lw, lh = bx + bw // 2 - 260, by + bh + 70, 520, 150
    panel(d, lx, ly, lw, lh, None)
    label(d, lx + 20, ly + 16, "~/.caddis/warden-ledger.jsonl", 19, TEAL)
    rows = [
        ('"from":"omp","body":"deny|git push --force origin main"', AMBER),
        ('"from":"little-coder","body":"allow|echo ok"', DIM),
        ('"from":"prime-agent","body":"allow|cargo test"', DIM),
    ]
    for i, (s, c) in enumerate(rows):
        label(d, lx + 20, ly + 50 + i * 30, s[:66], 17, c, bold=(c == AMBER))
    label(d, 60, H - 64, "the binary is spawned per call; the ledger on disk is the only state", 19)
    img.save(os.path.join(ASSETS, "diagram-architecture.png"))

# ── diagram 2: the verdict flow ──────────────────────────────────────────────

def draw_flow():
    W, H = 1600, 900
    img, d = canvas(W, H)
    d.text((60, 44), "THE VERDICT FLOW", font=mono(34, bold=True), fill=TEXT)
    label(d, 60, 92, "a tool call arrives as one length-prefixed frame; one JSON verdict leaves", 22)

    # input
    panel(d, 60, 180, 430, 190)
    label(d, 84, 200, "tool_call", 24, TEXT, bold=True)
    for i, s in enumerate(['tool 4', 'bash', 'command 28', 'git push --force origin main']):
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
        ('seq 1030', 'deny', AMBER),
        ('seq 1031', 'steer', TEAL),
        ('seq 1032', 'allow', DIM),
        ('seq 1033', 'allow', DIM),
    ]
    for i, (sq, vd, col) in enumerate(sample):
        y = 226 + i * 50
        d.line([1124, y + 38, 1516, y + 38], fill=EDGE, width=1)
        label(d, 1124, y, sq, 18, DIM)
        label(d, 1400, y, vd, 18, col, bold=True)
    label(d, 1124, 470, '"what did the agent do', 18, DIM)
    label(d, 1124, 496, ' last night?" - one grep', 18, DIM)
    arrow(d, 950, 700, 1100, 660)
    label(d, 60, H - 64, "unreadable verdict -> BLOCK (judgement fails closed)   binary missing -> allow, loudly", 19)
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
        ('$ onboard', DIM),
        ('onboard: binary installed at ~/.caddis/bin/caddis-warden', TEXT),
        ('onboard: PROOF - force-push DENIED (ledger seq 1120), echo ALLOWED.', AMBER),
    ]
    for i, (s, c) in enumerate(lines):
        label(d, 84, 512 + i * 34, s, 20, c, bold=(c == AMBER))
    img.save(os.path.join(ASSETS, "diagram-onboard.png"))

if __name__ == "__main__":
    os.makedirs(ASSETS, exist_ok=True)
    draw_logo()
    draw_arch()
    draw_flow()
    draw_onboard()
    print("rendered: logo.png, diagram-architecture.png, diagram-verdict.png, diagram-onboard.png")
