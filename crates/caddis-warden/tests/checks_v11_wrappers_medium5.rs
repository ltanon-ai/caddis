//! checks_v11_wrappers_medium5.rs — MEDIUM 5 from the fourth audit: eleven
//! more wrappers measured ALLOW, every one a name the registry never carried.
//!
//! The lesson of CARD-WARDEN-11 applies to the additions themselves: a name
//! lands WITH its grammar in the same tuple entry or not at all, value flags
//! only where the real tool's options are certain. Where a wrapper's own flag
//! surface is enormous (firejail) or unverified (torsocks), the entry carries
//! the BARE spelling and says so — an uncertain flag guess opens holes in the
//! other direction, which is worse.
//!
//! `chrt` and `flock` also join `timeout` as leading-operand runners: their
//! first positional is their own argument (priority, lock file), not the
//! wrapped command.

use caddis_warden::checks::git;

#[test]
fn the_eleven_measured_wrappers_do_not_hide_the_push() {
    for cmd in [
        "taskset -c 0 git push --force origin main",
        "taskset --cpu-list 0 git push --force origin main",
        "chrt -f 10 git push --force origin main",
        "flock /tmp/lk git push --force origin main",
        "flock -w 5 /tmp/lk git push --force origin main",
        "proxychains git push --force origin main",
        "proxychains -f /etc/pc.conf git push --force origin main",
        "torsocks git push --force origin main",
        "unbuffer git push --force origin main",
        "runuser -u root -- git push --force origin main",
        "runuser --user root git push --force origin main",
        "pkexec git push --force origin main",
        "strace -f git push --force origin main",
        "strace -o /tmp/trace.txt git push --force origin main",
        "strace -c git push --force origin main", // boolean -c: summary mode
        "systemd-run --scope --unit x git push --force origin main", // --unit takes x
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "a real wrapper spelling must not hide the push: {cmd}"
        );
    }
}

#[test]
fn the_wrapper_guard_holds_for_the_new_entries() {
    // `git` as docker's argument must stay ALLOW under every new wrapper —
    // and a boolean flag of each family must not swallow the command word.
    for cmd in [
        "taskset -c 0 docker run --rm git push --force origin main",
        "chrt -f 10 docker run --rm git push --force origin main",
        "flock /tmp/lk docker run --rm git push --force origin main",
        "strace -f docker run --rm git push --force origin main",
        "runuser -u root -- docker run --rm git push --force origin main",
        "systemd-run --scope docker run --rm git push --force origin main",
        // (Boolean-flag and value-flag controls live in the deny test above:
        // `strace -c git push ...` and `systemd-run --scope --unit x git
        // push ...` are REAL pushes under boolean/value flags.)
    ] {
        assert_eq!(
            git::force_push_to_protected(cmd),
            None,
            "`git` here is an ARGUMENT to docker (or a boolean control): {cmd}"
        );
    }
}
