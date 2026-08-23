#!/bin/sh
# checksums.sh — SHA256SUMS for the artifacts you are about to publish.
# Prints to stdout; a release pipes it into the SHA256SUMS file.
set -e
cd "$(dirname "$0")/.."
found=0
for f in target/release/caddis-warden target/release/caddis-warden.exe; do
    if [ -f "$f" ]; then
        sha256sum "$f"
        found=1
    fi
done
if [ "$found" = "0" ]; then
    echo "checksums: no release artifacts found - build first (cargo build --release)" >&2
    exit 1
fi
