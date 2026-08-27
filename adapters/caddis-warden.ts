// caddis-warden — the harness adapter. THE CONSCIOUSNESS'S NERVE, NOT ITS BRAIN.
//
// Deliberately thin and deliberately stupid: it marshals a tool call to the
// warden binary and applies the verdict. It holds NO policy. That is the whole
// update-resistance argument — harness APIs move fast, so the
// only thing exposed to API drift is this file. If v19 renames an event, this
// is ~10 lines of repair and the law in Rust never notices.
//
// Wire: length-prefixed OUT (arbitrary payloads cannot break a byte count),
// JSON IN (the binary is the producer; JSON.parse is stdlib here). Neither side
// hand-writes a parser for a format it does not control.
//
// ── FAILURE DOCTRINE, and the two cases are NOT the same ────────────────────
//   binary MISSING/unspawnable -> ALLOW, screaming. A deployment problem must
//        not brick the operator's tool at 3am. But a silently absent conscience
//        is exactly the failure he is most exposed to, so it is impossible to
//        miss: stderr + a UI notification on every call.
//   binary RAN but the reply is unreadable -> BLOCK. It judged and we cannot
//        read the judgement; trusting that is guessing. Judgement fails closed.

import { spawnSync } from "node:child_process";

// The INSTALLED binary, deliberately not `target/release`: a `cargo clean` in
// the workshop would otherwise disable the conscience silently, and "the guard
// quietly stopped existing" is the failure this whole crate is against.
// Install with: cargo build --release -p caddis-warden && cp the exe here.
const BIN =
  process.env.CADDIS_WARDEN_BIN ||
  `${process.env.USERPROFILE || process.env.HOME}/.caddis/bin/caddis-warden.exe`;

// Who is asking: one conscience serves several harnesses, and
// the shared ledger must be able to say WHICH one made each call. The onboarding
// script stamps this constant per agent; an unstamped copy keeps the built-in
// binary's default, so the file is safe to copy around unmodified.
const CALLER = "agent";

type Verdict = { verdict: string; reason: string; law: string; seq: number };

/** Length-prefixed request frame. Byte lengths, not character counts. */
function frame(fields: Record<string, string>): Buffer {
  const parts: Buffer[] = [];
  for (const name of ["tool", "command", "path", "content"]) {
    const body = Buffer.from(fields[name] ?? "", "utf8");
    parts.push(Buffer.from(`${name} ${body.length}\n`, "utf8"), body, Buffer.from("\n", "utf8"));
  }
  return Buffer.concat(parts);
}

const READ_ONLY = new Set(["read", "grep", "glob", "ls", "list"]);

/**
 * Pull the fields the law reasons about out of the tool call's `input` bag.
 *
 * ⚠ ONLY THE CONTENT BEING WRITTEN IS SCANNED — never `old_string`. An `edit`
 * carries the text it is REPLACING, and scanning that would deny an edit merely
 * for touching a line near an existing suppression: the warden would punish you
 * for cleaning up the very thing it dislikes.
 *
 * If a write-ish tool matches none of the known keys we fall back to the whole
 * input bag, so a renamed key degrades to over-scanning rather than to a silent
 * hole. A rule with a hole shaped like a key name is not a rule.
 */
function extract(toolName: string, input: Record<string, unknown>) {
  const s = (v: unknown) => (typeof v === "string" ? v : "");
  const command = s(input.command) || s(input.cmd);
  const path =
    s(input.path) || s(input.file_path) || s(input.filePath) || s(input.filename);
  let content =
    s(input.content) || s(input.new_string) || s(input.new_str) || s(input.text);

  // An `edit` commonly arrives as a UNION of param shapes (Replace | ReplaceBatch |
  // Patch | Hashline | ApplyPatch | Sloppy) — measured from its type defs, not
  // assumed. ReplaceBatch is `{path, edits: [{old_string, new_string}]}`, so
  // without this branch a batch edit matches none of the keys above, falls
  // through to the whole-input fallback, and drags every `old_string` into the
  // scan. The warden would then DENY an edit for REMOVING a suppression, which
  // is the precise opposite of what it wants to encourage.
  if (!content && Array.isArray(input.edits)) {
    content = input.edits
      .map((e: unknown) => {
        const r = (e ?? {}) as Record<string, unknown>;
        return s(r.new_string) || s(r.new_str) || s(r.content) || s(r.text);
      })
      .filter(Boolean)
      .join("\n");
  }

  if (!READ_ONLY.has(toolName) && !command && !content) {
    try {
      content = JSON.stringify(input) ?? "";
    } catch {
      content = "";
    }
  }
  return { tool: toolName, command, path, content };
}

function ask(fields: Record<string, string>): Verdict | "unspawnable" | "unreadable" {
  let res;
  try {
    res = spawnSync(BIN, [], {
      input: frame(fields),
      maxBuffer: 64 * 1024 * 1024,
      env: { ...process.env, CADDIS_WARDEN_FROM: CALLER },
    });
  } catch {
    return "unspawnable";
  }
  if (res.error || res.status === null) return "unspawnable";
  const out = (res.stdout?.toString("utf8") ?? "").trim();
  if (!out) return "unreadable";
  try {
    const v = JSON.parse(out) as Verdict;
    return typeof v?.verdict === "string" ? v : "unreadable";
  } catch {
    return "unreadable";
  }
}

export default function caddisWardenAdapter(pi: any) {
  // Laws owed to a tool call that was allowed but has something to say. Keyed
  // by toolCallId and DELETED on delivery, so a long session cannot grow this
  // without bound.
  const owed = new Map<string, string>();
  let warnedMissing = false;

  pi.on("tool_call", async (event: any, ctx: any) => {
    const fields = extract(String(event?.toolName ?? ""), event?.input ?? {});
    const v = ask(fields);

    if (v === "unspawnable") {
      if (!warnedMissing) {
        warnedMissing = true;
        console.error(
          `[caddis-warden] CONSCIENCE OFFLINE — binary not runnable at ${BIN}. ` +
            `Tools are running UNJUDGED. Build it: cargo build --release -p caddis-warden`
        );
        try {
          ctx?.ui?.notify?.("caddis-warden OFFLINE — tools are running unjudged", "error");
        } catch {}
      }
      return undefined; // allow, loudly
    }

    if (v === "unreadable") {
      return {
        block: true,
        reason:
          "caddis-warden ran but its verdict could not be read. Refusing to guess: " +
          "a judgement you cannot read is not an approval. Check the warden binary.",
      };
    }

    if (v.verdict === "deny") return { block: true, reason: v.reason };
    if (v.verdict === "steer" && v.law) {
      const id = String(event?.toolCallId ?? "");
      if (id) owed.set(id, v.law);
    }
    return undefined;
  });

  // A steer is delivered ON THE RESULT rather than as a block: the action was
  // legitimate, so it runs — and the law arrives attached to its outcome, which
  // is the moment it actually applies. This is the jit-laws mechanism: doctrine
  // when it is relevant, not at session start where it is read once and lost.
  pi.on("tool_result", async (event: any) => {
    const id = String(event?.toolCallId ?? "");
    const law = id && owed.get(id);
    if (!law) return undefined;
    owed.delete(id);
    const content = Array.isArray(event?.content) ? event.content : [];
    return { content: [...content, { type: "text", text: `\n[caddis-warden] ${law}` }] };
  });
}
