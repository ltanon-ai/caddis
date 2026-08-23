//! checks_v15_exe_and_execform.rs — Warden16to19Reviewer's re-measure of
//! 9e85f3c: two residuals, one in each direction.
//!
//! 1. `bash.exe -c "git push …"` — the reviewer measured bash.exe executing
//!    the string (BASH-EXE-RAN-STRING; PATH resolution). `bash.exe` is this
//!    estate's NATIVE spelling of bash, and the `.exe` strip only ran in the
//!    absolute-path branch — the bare form fell through and allowed.
//! 2. `env -S 'echo a; git push …'` — env -S execs the split string DIRECTLY
//!    (measured: prints `a; git push …`); no shell runs, so a `;` in the
//!    string is a literal argv character. Judging it with shell-line
//!    semantics DENIED a line where nothing pushy runs — a false deny, the
//!    wallpaper direction.

use caddis_warden::checks::git;

#[test]
fn the_windows_exe_spelling_of_a_shell_is_still_a_carrier() {
    for cmd in [
        "bash.exe -c \"git push --force origin main\"",
        "sh.exe -c 'git push --force origin main'",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "bash.exe is bash: the .exe suffix is Windows, not a different program: {cmd}"
        );
    }
    // Control: the suffix does not make anything a shell — notbash.exe is
    // not bash, and a relative ./bash.exe is still not an identity claim.
    assert_eq!(
        git::force_push_to_protected("notbash.exe -c 'git push --force origin main'"),
        None,
    );
    assert_eq!(
        git::force_push_to_protected("./bash.exe -c 'git push --force origin main'"),
        None,
    );
}

#[test]
fn env_dash_s_execs_argv_directly_not_a_shell_line() {
    // env -S splits and EXECS: the `;` is a literal argument to echo. Nothing
    // pushy runs, and the line must stay green.
    assert_eq!(
        git::force_push_to_protected("env -S 'echo a; git push --force origin main'"),
        None,
        "no shell runs; the semicolon is argv text, not an operator"
    );
    // Control: the direct form that really pushes still denies.
    assert!(
        git::force_push_to_protected("env -S 'git push --force origin main'").is_some(),
        "env -S 'git push …' execs git push directly"
    );
    // Control: flock -c and runuser -c pass their string TO A SHELL (their
    // man pages say so), so shell-line judgement stays correct there.
    assert!(
        git::force_push_to_protected("flock -c 'echo a; git push --force origin main'").is_some(),
        "flock -c hands the string to a shell; the second command runs"
    );
}
