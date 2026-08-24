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
# EVERY STEP IS VERIFIED RATHER THAN ASSUMED, because each unchecked one restores
# the same failure in a new costume: an unremoved destination makes `cp` nest
# again, and a copy that fails after the removal leaves no skill at all.
#
#   install-skill.sh <src-skill-dir> <dest> [<dest> ...]
#
# Exit: 0 at least one destination installed · 1 none installed · 2 usage.
set -u

if [ "$#" -lt 2 ]; then
    echo "usage: install-skill.sh <src-skill-dir> <dest> [<dest> ...]" >&2
    exit 2
fi

src="$1"
shift
if [ ! -f "$src/SKILL.md" ]; then
    echo "install-skill: '$src' carries no SKILL.md; nothing installed" >&2
    exit 1
fi
src_real="$(cd "$src" && pwd -P)"
leaf="$(basename "$src")"

installed=0
for dest in "$@"; do
    if [ -e "$dest" ] && [ ! -f "$dest/SKILL.md" ]; then
        echo "install-skill: WARNING — '$dest' exists and is not a caddis skill; left alone" >&2
        continue
    fi
    # A destination that resolves to the source would be deleted below and the
    # copy would then have nothing left to read.
    if [ -d "$dest" ] && [ "$(cd "$dest" && pwd -P)" = "$src_real" ]; then
        echo "install-skill: WARNING — '$dest' is the source itself; left alone" >&2
        continue
    fi
    if ! mkdir -p "$(dirname "$dest")"; then
        echo "install-skill: WARNING — cannot create the parent of '$dest'" >&2
        continue
    fi
    rm -rf -- "$dest"
    if [ -e "$dest" ]; then
        # Unremovable (in use, permissions). Copying now would nest into it and
        # still exit 0 — the exact bug this script was written to end.
        echo "install-skill: WARNING — could not clear '$dest'; left as it was" >&2
        continue
    fi
    if ! cp -r -- "$src" "$dest"; then
        echo "install-skill: WARNING — copy into '$dest' failed; it is now MISSING" >&2
        continue
    fi
    rm -rf -- "$dest/__pycache__"
    if [ ! -f "$dest/SKILL.md" ] || [ -e "$dest/$leaf" ]; then
        echo "install-skill: WARNING — '$dest' is not a usable skill dir after the copy" >&2
        continue
    fi
    echo "install-skill: installed -> $dest"
    installed=$((installed + 1))
done

if [ "$installed" -eq 0 ]; then
    echo "install-skill: nothing was installed" >&2
    exit 1
fi
exit 0
