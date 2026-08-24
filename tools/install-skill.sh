#!/usr/bin/env bash
# install-skill.sh — put the caddis agent skill into a harness skill directory.
#
# WHY THIS IS ITS OWN FILE. It has to be IDEMPOTENT, and idempotence is the one
# property a one-shot onboarding script never proves about itself: the self-proof
# runs on a fresh machine, where the destination does not exist yet. The re-run
# path — every update after the first, which ONBOARD.md tells you to perform — was
# therefore never exercised, and it was broken.
#
# `cp -r SRC DEST` copies INTO DEST when DEST already exists. The second onboard
# produced DEST/caddis/ and left the stale skill upstairs, while still reporting
# success: a success message over a no-op, which is the failure this project
# exists against. So the destination is REPLACED — but only one that proves it is
# ours by carrying a SKILL.md; anything else is left untouched and reported.
#
#   install-skill.sh <src-skill-dir> <dest> [<dest> ...]
set -u

src="${1:?usage: install-skill.sh <src-skill-dir> <dest> [<dest> ...]}"
shift
if [ ! -f "$src/SKILL.md" ]; then
    echo "install-skill: WARNING — '$src' carries no SKILL.md; nothing installed" >&2
    exit 1
fi

status=0
for dest in "$@"; do
    if [ -e "$dest" ] && [ ! -f "$dest/SKILL.md" ]; then
        echo "install-skill: WARNING — '$dest' exists and is not a caddis skill; left alone" >&2
        status=1
        continue
    fi
    mkdir -p "$(dirname "$dest")" 2>/dev/null || true
    rm -rf "$dest"
    if cp -r "$src" "$dest" 2>/dev/null; then
        rm -rf "$dest/__pycache__"
        echo "install-skill: installed -> $dest"
    else
        echo "install-skill: WARNING — could not install at '$dest'" >&2
        status=1
    fi
done
exit "$status"
