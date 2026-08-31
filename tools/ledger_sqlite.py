#!/usr/bin/env python3
"""CARD-LEDGER-DB-3: SQLite ledger helper. No rusqlite in the TCB."""
from __future__ import annotations

import argparse
import json
import os
import sqlite3
import sys
import time

SCHEMA = """
CREATE TABLE IF NOT EXISTS verdicts (
  seq INTEGER PRIMARY KEY,
  ts TEXT NOT NULL DEFAULT '',
  project TEXT NOT NULL DEFAULT '',
  from_agent TEXT NOT NULL DEFAULT '',
  type TEXT NOT NULL DEFAULT '',
  verdict TEXT NOT NULL DEFAULT '',
  body TEXT NOT NULL DEFAULT '',
  path TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS meta (
  k TEXT PRIMARY KEY,
  v TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_verdicts_project_seq ON verdicts(project, seq);
"""

SKIP = {
    "users",
    "ashpac",
    "scratch",
    "crates",
    "src",
    "tests",
    "target",
    "windows",
    "home",
}


def connect(db: str) -> sqlite3.Connection:
    parent = os.path.dirname(db)
    if parent:
        os.makedirs(parent, exist_ok=True)
    con = sqlite3.connect(db, timeout=30)
    con.execute("PRAGMA journal_mode=WAL")
    con.executescript(SCHEMA)
    con.execute("INSERT OR IGNORE INTO meta(k,v) VALUES('schema_version','1')")
    con.execute(
        "INSERT OR IGNORE INTO meta(k,v) VALUES('cutover','CARD-LEDGER-SPLIT-1')"
    )
    return con


def _skip_part(part: str) -> bool:
    if part.lower() in SKIP:
        return True
    if part.endswith((".rs", ".py", ".md", ".json", ".jsonl")):
        return True
    if len(part) == 2 and part.endswith(":"):
        return True
    return len(part) <= 1


def project_of(path: str, fallback: str = "") -> str:
    p = (path or "").replace("\\", "/")
    if "caddis-workshop" in p:
        return "caddis-workshop"
    parts = [x for x in p.split("/") if x and x not in (".", "..")]
    for part in reversed(parts[:-1] if parts else []):
        if not _skip_part(part):
            return part
    return fallback or "unknown"


def verdict_of(body: str, explicit: str = "") -> str:
    if explicit:
        return explicit.lower()
    head = (body or "").split("|", 1)[0].strip().lower()
    if head in ("allow", "deny", "steer"):
        return head
    return ""


def cmd_insert(db: str, payload: dict) -> None:
    con = connect(db)
    seq = payload.get("seq")
    if not seq:
        seq = con.execute("SELECT COALESCE(MAX(seq),0)+1 FROM verdicts").fetchone()[0]
    path = payload.get("path") or ""
    project = payload.get("project") or project_of(path, payload.get("cwd_project") or "")
    body = payload.get("body") or ""
    con.execute(
        "INSERT OR REPLACE INTO verdicts"
        "(seq,ts,project,from_agent,type,verdict,body,path) VALUES(?,?,?,?,?,?,?,?)",
        (
            int(seq),
            payload.get("ts") or "",
            project,
            payload.get("from") or "",
            payload.get("type") or "",
            verdict_of(body, payload.get("verdict") or ""),
            body,
            path,
        ),
    )
    con.commit()
    print(seq)


def cmd_get(db: str, seq: int) -> None:
    con = connect(db)
    row = con.execute(
        "SELECT seq,ts,project,from_agent,type,verdict,body,path "
        "FROM verdicts WHERE seq=?",
        (seq,),
    ).fetchone()
    if not row:
        sys.exit(1)
    keys = ["seq", "ts", "project", "from", "type", "verdict", "body", "path"]
    print(json.dumps(dict(zip(keys, row))))


def since_cutoff(since: str) -> int:
    now = int(time.time())
    if since.endswith("d") and since[:-1].isdigit():
        return now - int(since[:-1]) * 86400
    if since.endswith("h") and since[:-1].isdigit():
        return now - int(since[:-1]) * 3600
    if since.isdigit():
        return int(since)
    raise SystemExit(f"bad --since {since}")


def cmd_orient(db: str, project: str, since: str) -> None:
    cut = since_cutoff(since)
    con = connect(db)
    max_seq = con.execute("SELECT COALESCE(MAX(seq),0) FROM verdicts").fetchone()[0]
    win = "project=? AND CAST(ts AS INTEGER) >= ?"
    args = (project, cut)
    rows = con.execute(
        f"SELECT COUNT(*) FROM verdicts WHERE {win}", args
    ).fetchone()[0]
    counts = {"allow": 0, "deny": 0, "steer": 0}
    for v, n in con.execute(
        f"SELECT verdict, COUNT(*) FROM verdicts WHERE {win} GROUP BY verdict",
        args,
    ):
        if v in counts:
            counts[v] = n
    last_deny = con.execute(
        f"SELECT seq,ts,body FROM verdicts WHERE {win} AND verdict='deny' "
        "ORDER BY seq DESC LIMIT 1",
        args,
    ).fetchone()
    laws = con.execute(
        "SELECT seq,ts,body FROM verdicts WHERE CAST(ts AS INTEGER) >= ? "
        "AND body LIKE '%caddis-warden [%' "
        "ORDER BY seq DESC LIMIT 3",
        (cut,),
    ).fetchall()
    last20 = con.execute(
        f"SELECT seq,ts,verdict,type,from_agent,path FROM verdicts WHERE {win} "
        "ORDER BY seq DESC LIMIT 20",
        args,
    ).fetchall()
    print(f"PROJECT {project}")
    print(f"window_since={cut}")
    print(f"seq_max={max_seq}")
    print(f"rows={rows}")
    print(f"allow={counts['allow']} deny={counts['deny']} steer={counts['steer']}")
    if last_deny:
        body = (last_deny[2] or "").replace("\n", " ")[:180]
        print(f"last_deny: seq={last_deny[0]} ts={last_deny[1]} body={body}")
    else:
        print("last_deny: none")
    print("last_laws:")
    if not laws:
        print("  none")
    for seq, ts, body in laws:
        print(f"  seq={seq} ts={ts} {(body or '').replace(chr(10), ' ')[:160]}")
    print("last_20:")
    for seq, ts, verdict, typ, frm, path in last20:
        print(f"  {seq} {ts} {verdict} {typ} {frm} {path}")

def _row_from_line(line: str):
    if not line.startswith("{"):
        return None
    try:
        row = json.loads(line)
    except json.JSONDecodeError:
        return None
    body = row.get("body") or ""
    parts = body.split("|")
    path = parts[2] if len(parts) > 2 else ""
    return (
        int(row.get("seq") or 0),
        str(row.get("ts") or ""),
        project_of(path, "unknown"),
        str(row.get("from") or ""),
        str(row.get("type") or ""),
        verdict_of(body),
        body,
        path,
    )


def _flush(con, batch) -> int:
    con.executemany(
        "INSERT OR REPLACE INTO verdicts"
        "(seq,ts,project,from_agent,type,verdict,body,path) VALUES(?,?,?,?,?,?,?,?)",
        batch,
    )
    return len(batch)


def cmd_import(db: str, jsonl: str) -> None:
    con = connect(db)
    batch = []
    n = 0
    with open(jsonl, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            parsed = _row_from_line(line)
            if parsed is None:
                continue
            batch.append(parsed)
            if len(batch) >= 1000:
                n += _flush(con, batch)
                batch.clear()
    if batch:
        n += _flush(con, batch)
    con.commit()
    print(n)


def main() -> int:
    p = argparse.ArgumentParser(prog="ledger_sqlite")
    sub = p.add_subparsers(dest="cmd", required=True)
    ins = sub.add_parser("insert")
    ins.add_argument("--db", required=True)
    gt = sub.add_parser("get")
    gt.add_argument("--db", required=True)
    gt.add_argument("--seq", type=int, required=True)
    ori = sub.add_parser("orient")
    ori.add_argument("--db", required=True)
    ori.add_argument("--project", required=True)
    ori.add_argument("--since", default="90d")
    imp = sub.add_parser("import-jsonl")
    imp.add_argument("--db", required=True)
    imp.add_argument("--jsonl", required=True)
    args = p.parse_args()
    if args.cmd == "insert":
        payload = json.load(sys.stdin)
        cmd_insert(args.db, payload)
    elif args.cmd == "get":
        cmd_get(args.db, args.seq)
    elif args.cmd == "orient":
        cmd_orient(args.db, args.project, args.since)
    elif args.cmd == "import-jsonl":
        cmd_import(args.db, args.jsonl)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
