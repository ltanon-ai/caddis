"""mark.py — the logo mark and the full-width banner (the case + the spark)."""

import os

from .chrome import (
    AMBER,
    DIM,
    EDGE,
    TEAL,
    TEAL_DIM,
    TEXT,
    canvas,
    glow,
    label,
    mono,
    shard_ring,
    ui,
)

ASSETS = os.path.join(
    os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
    "assets",
)


def draw_logo():
    W = 1024
    img, d = canvas(W, W)
    cx, cy = W // 2, W // 2 - 30
    # hairline halo, two rings
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


def draw_banner():
    W, H = 2400, 640
    img, d = canvas(W, H)
    cx, cy = 320, H // 2
    shard_ring(d, cx, cy, r_out=225, r_in=148, n=17)
    glow(d, cx, cy, 21, AMBER)
    d.ellipse([cx - 24, cy - 24, cx + 24, cy + 24], fill=AMBER)
    d.text((620, 218), "caddis", font=ui(150), fill=TEXT)
    d.text((628, 408), "a conscience for coding agents", font=mono(52), fill=TEAL)
    label(d, 628, 486, "one binary - zero dependencies - any harness", 34, DIM)
    img.save(os.path.join(ASSETS, "banner.png"))
