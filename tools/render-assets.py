"""render-assets.py — regenerate every PNG in assets/ deterministically.

    python tools/render-assets.py

The renderer is split: assets/chrome.py (palette + primitives + the case
motif), assets/mark.py (logo + banner), assets/diagrams.py (the system
diagrams), assets/diagrams_records.py (the record-keeping diagrams).
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from assets.diagrams import draw_arch, draw_flow, draw_onboard
from assets.diagrams_records import draw_cards, draw_ledger, draw_memory
from assets.mark import draw_banner, draw_logo

if __name__ == "__main__":
    os.makedirs(os.path.join(os.path.dirname(__file__), "..", "assets"), exist_ok=True)
    draw_logo()
    draw_banner()
    draw_arch()
    draw_flow()
    draw_onboard()
    draw_ledger()
    draw_memory()
    draw_cards()
    print("rendered: logo, banner, architecture, verdict, onboard, ledger, memory, cards")
