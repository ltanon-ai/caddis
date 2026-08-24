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
| `git.push.force-to-protected` | force-push to `main`/`master` — the force flag OR a leading-plus refspec |
| `git.push.into-rewritten-repo` | publishing more history into a repo whose protected history was rewritten |
| `git.hooks.skipped` | `commit` with the hook-skip flag, long or `-n` <!-- nobs-allow: no-verify-flag — law documentation: naming the exact flag git.hooks.skipped denies; nothing here invokes it --> — skipping the hooks that are the only quality control here |
| `git.signing.bypassed` | the signing-off flags on `commit`/`push` — denied with their own reason, never the hooks' |
| `shell.skip-ci` | `[skip ci]` in a commit message — a marker with exactly one meaning |
| `shell.osv-no-resolve` | turning dependency resolution off so a scanner reports clean on a set it never examined |
| `fs.rmrf.protected-root` | `rm -rf` against absolute roots, `..` escapes, `.*` globs, NULL variable expansions, bare `*` at the workspace root — every flag spelling, through `sudo`/`env` wrapper descent |
| suppression-marker laws | skip markers smuggled into tracked files |
| secret literals | credential-shaped writes into files |
| sensitive paths | never-read-or-expose files, any tool, even read |
| unreadable own verdict | fail-closed: block, with the reason |

## SOFT findings (steer)

A steer does not block: the action was legitimate, so it runs — and the law
arrives attached to its **result**, which is the moment it applies. Every soft
finding that fired is carried; silently dropping one is how a channel stops
being worth reading.

| Law | Catches |
| --- | --- |
| `git.reset.discards-uncommitted` | `git reset --hard` with uncommitted work in the tree |
| `git.stage.blanket-in-shared-worktree` | `git add -A` / `add .` / `commit -a` — staging whatever is in the tree, in a worktree that may be shared with another writer |
| `fs.rmrf.wildcard` | bare `rm -rf *` in a directory the command `cd`'d into — the workspace-root case is the hard law's; this steer never stacks |
| `net.pipe-to-shell` | `curl`/`wget` piped into a shell — steers with a domain trustlist (rustup, Homebrew, bun named); everything else shows the exact URL |
| `shell.exit-code-through-pipe` | `$?` read after a pipeline reports the LAST process — a false green; `${PIPESTATUS[0]}` and `pipefail` stay silent |
| `shell.gate-chained-into-commit` | a gate chained with `&&` into `git commit` — the chain commits on red the moment the short-circuit is misread |
| `shell.git-show-piped-counter` | `git show rev:path` piped into a counter — a missing path exits 128 with empty stdout and the count reads as an authoritative 0 |
| `shell.process-query-self-match` | a process listing filtered by a literal the query itself contains — the query matches itself, dead reported alive |
| `shell.posix-tmp-across-python` | a POSIX `/tmp` path handed across the bash-to-Windows-Python boundary — the write and the read touch different files (fires on Windows only) |

## One documented cycle, start to finish

The comment law, as it actually happened:

1. **The defect class:** bash comments, seen by bash but not by the law —
   so a force-push hiding behind one was judged on tokens bash never
   runs, and a harmless line before one was denied for the same reason.
2. **RED first, both directions:** `git push # don't --force origin main`
   must allow (everything after `#` at a word start is a comment; the
   command is a bare `git push`), while a force-push before a comment,
   and on the line after one, must still deny. Pinned before any fix.
3. **The smallest fix:** teach the tokenizer what bash knows — `#` at a
   word start comments to end of line.
4. **The regression pin:** those tests live in
   `crates/caddis-warden/tests/checks_v9_comments.rs` forever.
   Re-introduce either defect and the suite names it.

Every fixture in `tests/` is one such cycle. Run them all:

```sh
cargo test --workspace
```

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
   `crates/caddis-warden/tests/`. 197 warden tests currently pin 25 hardening
   iterations; a regression is a failing name, not a debate.

Known honest limits are stated in the source, not hidden: the runner table is
enumerated, not derived ("which commands wrap another command" cannot be derived
from the token stream), and unlexable segments are not judged — a known gap,
recorded where the next reader will meet it.
