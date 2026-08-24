"""test_assets.py — the renderer's contract, smoke-tested: every draw
call writes a nonzero PNG into the assets directory it is pointed at.

The diagram split (CARD-0096) put diagrams.py and diagrams_records.py
under the sonar 80% new-coverage gate; these tests are what feeds it —
and what catches a draw function that silently stops saving its file.
"""

import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import assets.diagrams
import assets.diagrams_records
import assets.diagrams_work


def _draws(tmp_path, monkeypatch, module, calls):
    monkeypatch.setattr(module, "ASSETS", str(tmp_path), raising=False)
    from PIL import Image

    for fn_name, out_name in calls:
        getattr(module, fn_name)()
        out = tmp_path / out_name
        assert out.is_file() and out.stat().st_size > 0, f"{fn_name} wrote nothing"
        with Image.open(out) as img:
            assert img.width == 1600, f"{out_name} is {img.size}, not 1600 wide"


def test_system_diagrams_render(tmp_path, monkeypatch):
    _draws(
        tmp_path,
        monkeypatch,
        assets.diagrams,
        (
            ("draw_arch", "diagram-architecture.png"),
            ("draw_flow", "diagram-verdict.png"),
            ("draw_onboard", "diagram-onboard.png"),
        ),
    )


def test_record_diagrams_render(tmp_path, monkeypatch):
    _draws(
        tmp_path,
        monkeypatch,
        assets.diagrams_records,
        (
            ("draw_ledger", "diagram-ledger.png"),
            ("draw_memory", "diagram-memory.png"),
            ("draw_cards", "diagram-cards.png"),
        ),
    )



def test_work_diagrams_render(tmp_path, monkeypatch):
    _draws(
        tmp_path,
        monkeypatch,
        assets.diagrams_work,
        (
            ("draw_card_anatomy", "diagram-card-anatomy.png"),
            ("draw_ladder", "diagram-ladder.png"),
            ("draw_tree", "diagram-tree.png"),
        ),
    )