# Threat model

Caddis is a **policy guard and audit layer** — not a sandbox, not a
privilege boundary, not a substitute for OS-level control. This file states
exactly what it defends, against whom, and where its protection ends. If a
claim anywhere in this repository sounds stronger than this file, this file
wins.

## What CADDIS is

A small law engine that a harness calls, through an adapter you installed,
before executing a tool call. It answers *allow / steer / deny* with a
reason, and appends every verdict to a local ledger.

## What CADDIS is not

- **Not an OS sandbox.** It has no process isolation, no seccomp, no
  namespaces, no privilege separation. It runs as your user and its
  judgements are only as binding as the harness's willingness to honor them.
- **Not arbitrary-code analysis.** The law parses shell. Code *inside* a
  heredoc, a quoted string, or an embedded program (Python, JS, anything a
  tool executes later) is payload, not commands, and is out of scope.
- **Not tamper-proof storage.** The ledger is append-only by program
  behavior; any process running as the same OS user can edit or delete it.

## The protected asset

Your shared history, your files, and your secrets — from **accidents by the
agent you attached**: the plausible-looking force-push, the credential
pasted into a file, the bypass marker smuggled into code. The adversary is
a competent but error-prone model, not a determined human attacker with
code execution.

## Trust assumptions (if any of these is false, the protection is gone)

1. **The harness routes the calls.** Caddis judges only what arrives through
   the adapter. If the agent can spawn a process outside the harness, use
   another execution path, or call tools the adapter does not intercept,
   those calls are never judged.
2. **The adapter is installed and enabled.** Removing the hook, renaming the
   extension, or pointing `CADDIS_WARDEN_BIN` elsewhere silently disarms
   everything. The loud fail-open warning covers an *absent* binary, not a
   *deliberately removed* integration.
3. **The binary is the one you built.** Anyone who can replace
   `~/.caddis/bin/caddis-warden` controls the verdicts. See
   [DISTRIBUTION.md](DISTRIBUTION.md) for verifying what you install.

## Attack classes, in scope

| Class | Example | Caught by |
| --- | --- | --- |
| history rewrite | force-push to a protected branch | HARD deny |
| credential writes | key-shaped literals into files | HARD deny |
| refspec force | push `origin +branch` with no force flag | HARD deny |
| gate suppression | skip markers smuggled into tracked files | HARD deny |
| sensitive reads | never-read-or-expose paths, any tool | HARD deny |
| doctrine gaps | legitimate call, missing context | steer on result |

## Attack classes, out of scope

- Embedded code executed by another interpreter (see above).
- Calls that bypass the harness tool surface entirely.
- A compromised adapter or binary (trust assumption 2 and 3).
- An attacker with your OS user — this includes editing the ledger
  retroactively: sequence numbers prove order *as written*, they are not a
  cryptographic chain.

## Failure doctrine (deliberate trade-off)

- **Binary missing or unspawnable → allow, loudly.** Availability wins: a
  deployment problem must not brick the agent at 3am. The warning is
  impossible to miss but it is still a warning — during it, calls run
  unjudged.
- **Binary ran but the verdict is unreadable → block.** Judgement fails
  closed; a judgement you cannot read is not an approval.

## Ledger standing

The ledger is **observability, not security evidence**. It is honest about
what happened while it was running: `seq: 0` marks an unrecorded decision
loudly, and a gap in sequence numbers means rows were not written at that
time. It does not prove the file was never edited — same-user processes can
rewrite it. Stronger guarantees (hash-chaining, signing) are a separate,
deliberate build; if you need them, they do not exist here yet.
