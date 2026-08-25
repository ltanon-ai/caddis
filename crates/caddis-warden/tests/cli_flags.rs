//! cli_flags.rs — the human-facing arguments.
//!
//! Every invocation used to fall through to "read a request frame from stdin",
//! so the first command a new user types (`--version`) answered with a DENIAL
//! and exit 0. A conscience whose own CLI cannot say what it is has no standing
//! to say what it judged.
//!
//! The FRAME path is pinned here too, because it is the one that must NOT move:
//! adapters pass no arguments and read the verdict from stdout, ignoring the
//! exit code.

use std::io::Write;
use std::process::{Command, Stdio};

fn run(args: &[&str], stdin: Option<&[u8]>) -> (String, String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_caddis-warden"))
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary must spawn");
    if let Some(bytes) = stdin {
        child
            .stdin
            .as_mut()
            .expect("stdin was piped")
            .write_all(bytes)
            .expect("frame written");
    }
    let out = child.wait_with_output().expect("the binary must finish");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn version_says_what_this_is_and_succeeds() {
    let (stdout, _stderr, code) = run(&["--version"], None);
    assert_eq!(code, 0, "asking the version is not an error: {stdout}");
    assert!(stdout.contains("caddis-warden"), "stdout was: {stdout}");
    assert!(
        !stdout.contains("deny"),
        "a version request must not be answered with a verdict: {stdout}"
    );
}

#[test]
fn help_prints_usage_and_succeeds() {
    let (stdout, _stderr, code) = run(&["--help"], None);
    assert_eq!(code, 0);
    assert!(stdout.to_lowercase().contains("usage"), "stdout: {stdout}");
    assert!(!stdout.contains("deny"), "stdout: {stdout}");
}

#[test]
fn an_unknown_argument_is_a_usage_error_never_a_denial() {
    let (stdout, stderr, code) = run(&["--frobnicate"], None);
    assert_eq!(
        code, 2,
        "an unknown flag must fail loudly: {stdout}{stderr}"
    );
    assert!(
        !stdout.contains("verdict"),
        "no verdict may be emitted for a misuse: {stdout}"
    );
    assert!(stderr.to_lowercase().contains("usage"), "stderr: {stderr}");
}

#[test]
fn the_frame_path_is_unchanged() {
    // No arguments, one frame on stdin: exactly what an adapter does. A denial
    // still exits 0, because the adapter reads the verdict and not the code.
    let frame = b"tool 4\nbash\ncommand 29\ngit push --force origin main\npath 0\n\ncontent 0\n\n";
    let (stdout, _stderr, code) = run(&[], Some(frame));
    assert_eq!(code, 0, "the frame path must keep exiting 0: {stdout}");
    assert!(stdout.contains("\"verdict\":\"deny\""), "stdout: {stdout}");
}
