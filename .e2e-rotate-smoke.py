"""Slice-A CLI e2e smoke: seed a sandbox home, serve /models from a local
stub (401 unauthenticated / 200 with bearer), run `rotate` 3x through the
REAL binary, assert the ruled outcomes: live lift on auth, unprobeable x3
transition + ONE alert, pure-JSON stdout, rc law."""
import json, os, subprocess, sys, tempfile, threading, time
from http.server import BaseHTTPRequestHandler, HTTPServer

BIN = r"E:\ClaudeToolbox\caddis\target\debug\caddis-deliberate.exe"
KEY = "e2e-key-42"

class Stub(BaseHTTPRequestHandler):
    def do_GET(self):
        if not self.path.endswith("/models"):
            self.send_response(404); self.end_headers(); return
        auth = self.headers.get("Authorization", "")
        if auth == f"Bearer {KEY}":
            body = b'{"data":[{"id":"stub-model"}]}'
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers(); self.wfile.write(body)
        else:
            self.send_response(401)
            self.send_header("Content-Length", "0")
            self.end_headers()
    def log_message(self, format, *args):
        pass

srv = HTTPServer(("127.0.0.1", 0), Stub)
port = srv.server_address[1]
threading.Thread(target=srv.serve_forever, daemon=True).start()

tmp = tempfile.mkdtemp(prefix="caddis-rotate-e2e-")
home = os.path.join(tmp, "home")
keyfile = os.path.join(tmp, "vault-key")
with open(keyfile, "w") as f: f.write(KEY + "\n")

catalog = {"providers": {
    "authprov": {
        "api": f"http://127.0.0.1:{port}/v1",
        "baseUrl": f"http://127.0.0.1:{port}/v1",
        "apiKey": keyfile,
        "models": [{"id": "m1", "contextWindow": 8192, "maxTokens": 4096}],
    },
    "blankprov": {
        "api": f"http://127.0.0.1:{port}/v2",
        "baseUrl": f"http://127.0.0.1:{port}/v2",
        "models": [{"id": "m2", "contextWindow": 8192, "maxTokens": 4096}],
    },
}}
models_path = os.path.join(tmp, "models.json")
with open(models_path, "w") as f: json.dump(catalog, f)

def run(*args):
    p = subprocess.run([BIN, *args], capture_output=True, text=True, timeout=120)
    return p.returncode, p.stdout.strip(), p.stderr.strip()

# Seed through the real CLI; authprov carries a PATH-LIKE apiKey (the
# vault-path law) so it probes authenticated; blankprov probes honest 401.
rc, out, err = run("seed", "--models", models_path, "--home", home)
print("seed rc", rc, "err:", err[:200])
assert rc == 0, "seed failed"

# Rotation 1: authed seat 200 -> Live card; blank-auth seat 401 -> streak 1.
rc, out, err = run("rotate", "--home", home)
rep = json.loads(out)
print("rotate1 rc", rc, "live", rep["live"], "unprobeable", rep["unprobeable"],
      "cards", rep["cards_appended"], "probed", rep["probed"])
assert rc == 0
assert rep["live"] == 1 and rep["unprobeable"] == 1 and rep["cards_appended"] == 1

# Rotation 2: Live seat not due (hourly cadence); blank seat streak 2, quiet.
rc, out, err = run("rotate", "--home", home)
rep2 = json.loads(out)
print("rotate2 rc", rc, "live", rep2["live"], "unprobeable", rep2["unprobeable"],
      "alerts", rep2["alerts"])
assert rc == 0 and rep2["alerts"] == [] and rep2["cards_appended"] == 0

# Rotation 3: the Q6 transition — ONE alert, ONE unprobeable card.
rc, out, err = run("rotate", "--home", home)
rep3 = json.loads(out)
print("rotate3 rc", rc, "alerts", rep3["alerts"], "cards", rep3["cards_appended"])
assert rc == 0 and len(rep3["alerts"]) == 1 and rep3["cards_appended"] == 1

# The view carries census-visible truth.
rc, out, err = run("view", "--home", home)
view = json.loads(out)
states = {s["id"]: s["state"] for s in view["seats"]}
print("view states:", states)
assert states["authprov/m1"] == "live"
assert states["blankprov/m2"] == "unprobeable"

# (Full auth landing through the edits path rides slice C on the real home.)

# rc law: nothing-due shape is unreachable here (probing seats stay due);
# verify rc2 defect on a young held lock:
with open(os.path.join(home, "rotate.lock"), "w") as f:
    f.write(json.dumps({"pid": 1, "started_epoch_s": int(time.time())}) + "\n")
rc, out, err = run("rotate", "--home", home)
print("lock-held rc", rc, "err:", err[:160])
assert rc == 2 and "held" in err

srv.shutdown()
print("E2E SMOKE PASS")
