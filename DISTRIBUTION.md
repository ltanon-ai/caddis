# Distribution and installation

Caddis is a security-adjacent tool: read this before running anything from
this repository. The trust boundary of an install is the artifact you
execute — verify it, or build it yourself from a commit you can read.

## Requirements — split honestly

| Phase | Needs |
| --- | --- |
| Runtime | nothing but the binary (~/.caddis/bin/caddis-warden) |
| Build | Rust toolchain (cargo), a C linker for your platform |
| Clone | git |
| Claude Code hook adapter | Python 3.8+ |
| Extension adapters | a harness that loads TypeScript extensions |

"One binary, zero dependencies" is a *runtime* claim: no runtime libraries
beyond the OS, no cloud, no background services. Building from source needs
the Rust toolchain; that is not a dependency of the product.

## What an install puts where

| Path | What |
| --- | --- |
| `~/.caddis/bin/caddis-warden[.exe]` | the law engine binary |
| `~/.caddis/warden-ledger.jsonl` | the verdict ledger (created on first run) |
| `~/.caddis/lanes.json` | optional cwd→label map (Claude Code adapter) |
| your harness's extension/hook dir | one adapter file |

## Verifying what you install

Today the reproducible path is: **build from a commit you can read**.

```sh
git clone https://github.com/ltanon-ai/caddis.git
cd caddis
git log --oneline -5          # know exactly what tree you are building
cargo build --release
tools/checksums.sh            # prints SHA256SUMS for what you built
./onboard <agent-name>        # install AND self-prove (deny + ledger row)
```

`onboard` fails unless the engine denies a force-push and the ledger row
exists — an install that cannot show you a denial is not an install.

### Verifying a release binary

Releases publish the Windows x64 binary and a `SHA256SUMS` file. Verify
before installing:

```sh
sha256sum -c SHA256SUMS
```

Checksums authenticate the download against the release page, not against
a signing key — the release page itself is trusted because it is your
GitHub. Binary signing (minisign/sigstore) is the next step after
releases prove their shape; it is deliberately not promised before it
ships. Other platforms: build from the tagged source and run onboard.

## Updating and rolling back

```sh
# before updating, keep the working binary — it IS your rollback
cp ~/.caddis/bin/caddis-warden ~/.caddis/bin/caddis-warden.prev
git pull && cargo build --release && cp target/release/caddis-warden ~/.caddis/bin/
./onboard <agent-name>        # re-prove after every update
# rollback: copy .prev back and re-run onboard
```

An update that fails the self-proof is not installed — restore `.prev`.

## Removal

1. Remove the adapter: delete the extension/hook file and its registration
   in your harness settings.
2. Delete `~/.caddis/bin/` and (if you want the history gone)
   `~/.caddis/warden-ledger.jsonl`. The ledger is the only record; the
   engine runs no background process and leaves nothing else behind.

## Platform status

Universal by construction: pure std Rust, no OS-specific dependencies, and
a CI matrix that builds, tests, and runs the onboard self-proof on **every
push, on all three platforms**.

| Platform | State |
| --- | --- |
| Windows (x64) | built and tested in CI; developed here |
| Linux (x64) | built, tested, self-proven in CI (GitHub matrix) |
| macOS (arm64/x64) | built, tested, self-proven in CI (GitHub matrix) |

Releases carry per-platform binaries named `caddis-warden-<OS>-x64[.exe]`
plus a `SHA256SUMS` covering them all.

## Release plan (requires explicit approval per release)

Tag `vX.Y.Z` → build binaries for the matrix above → generate
`SHA256SUMS` (tools/checksums.sh) → GitHub release with artifacts +
checksums → update this document's verify section to point at them.
Signing/provenance (sigstore or minisign) is the next step after the first
release exists; it is deliberately not promised before it ships.
