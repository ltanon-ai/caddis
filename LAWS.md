# The laws

What the warden denies, what it steers, and how a law is born.

## The order the law runs in

Order is law, not implementation detail:

1. **Sensitive paths, first, for every tool.** Files the contract names
   never-read-or-expose (credentials, vaults, session histories). Even a `read`
   is denied here — the one place reading is not free. 2. **Read-only tools
   leave early.** Reading is never taxed; scanning a read's payload would deny a
   `grep` for the very markers this engine exists to find. The warden must not
   forbid auditing itself. 3. **Suppression markers.** Writing skip/bypass
   markers into code is denied. Prose (`.md`, `.txt`, `.rst`, `.adoc`) is exempt
   — a guard must not forbid documenting its own rules — but the exemption is
   granted by RECOGNITION: config files and anything unrecognized stay fully in
   scope. The escape hatch is line-scoped, never file-wide. 4. **Secret
   literals.** A token-shaped value after `key =` and friends is denied before
   it lands in a file. 5. **The command registry.** Tokenize the shell command,
   descend wrappers (`sudo`, `env`, `nohup`, `timeout`, … — each with its own
   per-runner flag grammar, because `sudo -S` is boolean while `env -S` takes a
   string), resolve what will ACTUALLY run, and match it against the named laws.

## HARD findings (deny)

The force-push law catches the leading-plus refspec too (force with no force
*flag*), and does not misfire on branch names that merely contain "main".
The suppression laws exempt prose — a guard must not forbid documenting its
own rules — but the escape hatch is line-scoped, never file-wide.

| Law | Catches |
| --- | --- |
| `git.push.force-to-protected` | force-push to `main`/`master` |
| suppression-marker laws | skip markers smuggled into tracked files |
| secret literals | credential-shaped writes into files |
| sensitive paths | never-read-or-expose files, any tool, even read |
| unreadable own verdict | fail-closed: block, with the reason |

## SOFT findings (steer)

A steer does not block: the action was legitimate, so it runs — and the law
arrives attached to its **result**, which is the moment it applies. Every soft
finding that fired is carried; silently dropping one is how a channel stops
being worth reading.

## Known boundaries, stated rather than implied

- **The law parses shell.** Code *inside* a heredoc, quoted string or embedded
  program is payload, not commands — a force-push expressed as, say, embedded
  Python (`subprocess.run([...])` inside a heredoc) is out of scope today. A
  heredoc is exactly where an agent puts code, so assume this boundary. -
  **Write/Edit content is judged, not persisted.** The ledger row records the
  path; what was written stays out of the audit (size and secrets).

## How a law is added

Red-first, measured:

1. **Find the hole with a test, not an opinion.** Write the failing test that
   proves the bypass or the false positive. A claim about shell behavior is
   measured against the real tool (`getopt`, `bash`, `env`) before it becomes a
   rule — several laws exist because a "fix" was measured against the real
   binary and lost. 2. **The smallest fix that closes it.** Grammar goes in the
   grammar modules (`lexer`, `cmdline`, `runners`, `registry`, `positions`,
   `gitgrammar`); policies go in `law.rs`. One law, one name, both directions:
   it must catch the hole AND stop denying what it never had jurisdiction over.
   3. **Tests pin it forever.** Each law lands with its fixture file in
   `crates/caddis-warden/tests/`. 170+ tests currently pin 25 hardening
   iterations; a regression is a failing name, not a debate.

Known honest limits are stated in the source, not hidden: the runner table is
enumerated, not derived ("which commands wrap another command" cannot be derived
from the token stream), and unlexable segments are not judged — a known gap,
recorded where the next reader will meet it.
