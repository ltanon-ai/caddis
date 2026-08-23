//! registry.rs — the runner table: which commands wrap another command, and
//! which of their flags consume the next token.
//!
//! Split out of `runners.rs` under the 280-line file law when the table grew
//! past the cap. The seam is clean: this file is DATA (one tuple per runner,
//! name beside its grammar so neither can be added without the other), and
//! `runners.rs` is the CODE that walks a prefix past its entry.

/// Runner prefixes, and the flags of theirs that consume the next token.
///
/// ⛔ THE TWO-REGISTRY LESSON (CARD-WARDEN-11). For three cards this was a
/// NAME-only list beside a separate flag `match`, so every wrapper added
/// after an audit measured it landed with an empty flag table: recognised,
/// its value-taking flags read as boolean, its own argument eaten as the
/// command word. Audit 4 measured five such ALLOW holes at one head. The
/// tuple form is the fix for the SHAPE: a runner and its grammar are decided
/// in the same entry.
///
/// ⛔ PER-RUNNER, NOT FLAT (AUDIT 2). The same letter differs per runner:
/// `sudo -p` takes a prompt, `command -p` is boolean; `sudo -S` is boolean
/// while `env -S` takes a string. A flat list cannot be right.
///
/// ⚠ THIS LIST IS A GUESS, NOT A DERIVATION, AND THAT IS THE UNSOLVED PART.
/// Four separate audits have each found wrappers or wrapper grammar missing
/// from it — "which commands wrap another command" cannot be derived from the
/// token stream; it can only be enumerated. The alternative (treat any
/// `git <subcommand>` anywhere in a segment as the command) would close the
/// whole class at the cost of firing on `docker run git push`, a false
/// positive on a DENY-class gate. That trade is not mine to make quietly —
/// recorded here as an open design question rather than silently chosen.
/// Where a wrapper's own flag surface is enormous (firejail) or unverified
/// (torsocks), the entry carries the BARE spelling only, stated in place: an
/// uncertain flag guess opens holes in the other direction.
///
/// `xargs` is deliberately EXCLUDED: it reads its arguments from stdin and
/// `-I{}` rewrites them, so the tokens on the line are not the command that
/// runs. See `checks_v6_regress.rs::xargs_is_not_treated_as_a_wrapper_by_design`.
pub(crate) const RUNNERS: &[(&str, &[&str], &[&str])] = &[
    // sudo(8). `-S` is BOOLEAN here (password from stdin) while the same
    // letter is value-taking for env — one reason the table is per-runner.
    // `-a/--auth-type` and `-D/--chdir` (sudo 1.9+) were added after the
    // CARD-WARDEN-11 reviewer measured them missing.
    (
        "sudo",
        &[
            "-u",
            "--user",
            "-g",
            "--group",
            "-p",
            "--prompt",
            "-a",
            "--auth-type",
            "-C",
            "--close-from",
            "-D",
            "--chdir",
            "-U",
            "--other-user",
            "-T",
            "--command-timeout",
            "-R",
            "--chroot",
            "-r",
            "--role",
            "-t",
            "--type",
        ],
        &[],
    ),
    // env(1): -u NAME, -C DIR, -S STRING. `NAME=value` pairs after the flags
    // are skipped by the caller's assignment skip, not by this table.
    (
        "env",
        &["-u", "--unset", "-C", "--chdir", "-S", "--split-string"],
        &["-S", "--split-string"],
    ),
    // nohup(1) and command(1p) carry no value-taking flags.
    ("nohup", &[], &[]),
    ("command", &[], &[]),
    // time(1): only GNU time takes values (-f FORMAT, -o FILE); the shell
    // builtin's lone -p is boolean and absent from the table on purpose.
    ("time", &["-f", "--format", "-o", "--output"], &[]),
    // exec is a shell builtin, not a tool: it REPLACES the shell with the
    // command, which is wrapping in the purest sense. `-a` sets argv[0].
    ("exec", &["-a"], &[]),
    // timeout(1): -s SIGNAL / -k DURATION precede the mandatory duration
    // operand handled by `takes_leading_operand`.
    ("timeout", &["-s", "--signal", "-k", "--kill-after"], &[]),
    // nice(1): the adjustment is -n/--adjustment (or old-style `-12`), never
    // a positional — see `takes_leading_operand`.
    ("nice", &["-n", "--adjustment"], &[]),
    // doas(1) has no long options.
    ("doas", &["-u", "-C", "-a"], &[]),
    // setsid(1): -c/-f/-q/-w are all boolean.
    ("setsid", &[], &[]),
    // stdbuf(1) REQUIRES at least one of these before the command.
    (
        "stdbuf",
        &["-i", "--input", "-o", "--output", "-e", "--error"],
        &[],
    ),
    // ionice(1): -c/-n wrap a command; -p/-P retune an already-running one.
    (
        "ionice",
        &[
            "-c", "--class", "-n", "--level", "-p", "--pid", "-P", "--pgid",
        ],
        &[],
    ),
    // taskset(1): -c CPU-LIST wraps a command; -p retunes a running pid.
    ("taskset", &["-c", "--cpu-list", "-p", "--pid"], &[]),
    // chrt(1): the PRIORITY is a leading positional operand before the
    // wrapped command; -p retunes a running pid.
    ("chrt", &["-p", "--pid"], &[]),
    // flock(1): the LOCK FILE is a leading positional operand; -c/--command
    // carries a command STRING a shell executes — the third column.
    (
        "flock",
        &[
            "-w",
            "--wait",
            "-E",
            "--conflict-exit-code",
            "-c",
            "--command",
        ],
        &["-c", "--command"],
    ),
    ("proxychains", &["-f"], &[]),
    ("torsocks", &[], &[]),
    ("unbuffer", &[], &[]),
    // runuser(1): like sudo, minus the security theatre.
    (
        "runuser",
        &[
            "-u",
            "--user",
            "-g",
            "--group",
            "-G",
            "--supp-group",
            "-c",
            "--command",
            "-s",
            "--shell",
        ],
        &["-c", "--command"],
    ),
    ("pkexec", &["--user"], &[]),
    // strace(1): -o FILE, -e EXPR, -s SIZE, -S SORT, -p PID take values;
    // -f/-c are boolean. The full surface is vast — these are the value
    // forms that precede a wrapped command in real use.
    ("strace", &["-o", "--output", "-e", "-s", "-S", "-p"], &[]),
    // systemd-run(1): -p/--property repeat, --slice/--unit/--uid/--gid and
    // --working-directory take values; --scope/--wait are boolean.
    (
        "systemd-run",
        &[
            "-p",
            "--property",
            "--slice",
            "--unit",
            "--uid",
            "--gid",
            "--working-directory",
        ],
        &[],
    ),
    ("firejail", &[], &[]),
];

/// The registry's own `&'static` name for a runner token — the identity a
/// collected string is tagged with. The `.exe` suffix is Windows spelling
/// (env.exe IS env, PATH-resolved), stripped before lookup exactly as the
/// carrier rule strips it.
pub(super) fn runner_name(token: &str) -> Option<&'static str> {
    let token = token.strip_suffix(".exe").unwrap_or(token);
    RUNNERS
        .iter()
        .find(|(name, _, _)| *name == token)
        .map(|(name, _, _)| *name)
}

/// The full grammar of a known runner prefix — (value-taking flags,
/// string-carrying flags) — or `None` for any other token. Membership and
/// grammar answer through ONE registry, so a name can never be added without
/// its flags being decided in the same breath.
pub(super) fn runner_spec(
    token: &str,
) -> Option<(&'static [&'static str], &'static [&'static str])> {
    RUNNERS
        .iter()
        .find(|(name, _, _)| *name == token)
        .map(|(_, values, strings)| (*values, *strings))
}
