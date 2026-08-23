"""chrome.py — palette, fonts, canvas primitives, the shared diagram chrome."""

import math
import os

from PIL import Image, ImageDraw, ImageFont

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
    d._image.paste(
        Image.alpha_composite(d._image.convert("RGBA"), overlay).convert("RGB"), (0, 0)
    )


def shard_ring(d, cx, cy, r_out, r_in, n, seed=7):
    """n stones laid TANGENTIALLY into a ring — a caddis case, shingle by
    shingle. Radial teeth read as a sawblade no matter how much you jitter
    them (measured twice in review, v0/v1); found materials lie ALONG the
    current, not pointing out of it."""
    state = seed

    def rnd():
        nonlocal state
        state = (state * 1103515245 + 12345) % (2**31)
        return state / 2**31

    a = -math.pi / 2
    circ = 2 * math.pi * ((r_in + r_out) / 2)
    for _ in range(n):
        pitch = circ / n
        a += (pitch / ((r_in + r_out) / 2)) * (0.82 + 0.36 * rnd())
        L = pitch * (1.25 + 0.45 * rnd())
        W = (r_out - r_in) * (0.42 + 0.34 * rnd())
        mid_r = (r_in + r_out) * (0.485 + 0.035 * rnd())
        t = a + math.pi / 2 + (rnd() - 0.5) * 0.35
        ux, uy = math.cos(t), math.sin(t)
        vx, vy = -uy, ux
        mx, my = cx + mid_r * math.cos(a), cy + mid_r * math.sin(a)
        hL, sk = L / 2, W * 0.35 * (rnd() - 0.5)
        pts = [
            (
                mx + ux * -hL + vx * -W / 2 + uy * sk,
                my + uy * -hL + vy * -W / 2 - ux * sk,
            ),
            (mx + ux * hL + vx * -W / 2, my + uy * hL + vy * -W / 2),
            (mx + ux * hL + vx * W / 2, my + uy * hL + vy * W / 2),
            (
                mx + ux * -hL + vx * W / 2 - uy * sk,
                my + uy * -hL + vy * W / 2 + ux * sk,
            ),
        ]
        shade = 0.72 + 0.28 * rnd()
        col = tuple(int(c * shade) for c in TEAL)
        d.polygon(pts, fill=col)
        d.line(pts + [pts[0]], fill=INK, width=3)


def panel(d, x, y, w, h, title=None, accent=None):
    d.rounded_rectangle(
        [x, y, x + w, y + h], radius=10, fill=PANEL, outline=EDGE, width=2
    )
    if accent:
        d.rounded_rectangle([x, y, x + w, y + 6], radius=3, fill=accent)
    if title:
        d.text((x + 18, y + 18), title, font=mono(24, bold=True), fill=TEXT)


def arrow(d, x1, y1, x2, y2, color=TEAL_DIM, w=3):
    d.line([x1, y1, x2, y2], fill=color, width=w)
    ang = math.atan2(y2 - y1, x2 - x1)
    L = 12
    d.polygon(
        [
            (x2, y2),
            (x2 - L * math.cos(ang - 0.42), y2 - L * math.sin(ang - 0.42)),
            (x2 - L * math.cos(ang + 0.42), y2 - L * math.sin(ang + 0.42)),
        ],
        fill=color,
    )


def label(d, x, y, s, size=20, color=DIM, bold=False):
    d.text((x, y), s, font=mono(size, bold=bold), fill=color)
