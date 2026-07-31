#![cfg(unix)]

use std::{ffi::OsStr, os::unix::ffi::OsStrExt, process::Command};

fn run(variables: Vec<(&str, &OsStr)>) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_apmpr"));
    command.arg("help").env_clear();
    for (name, value) in variables {
        command.env(name, value);
    }
    command.output().expect("the CLI binary should run")
}

/// Covers the guard's *placement* in `run()`, not only the reporting helper. Deleting the
/// call would leave every command silently ignoring a renamed variable while a unit test of
/// the helper still passed.
#[test]
fn the_binary_refuses_a_stale_environment_and_never_prints_its_value() {
    let refused = run(vec![
        ("SWITCHYARD_ROUTER_TOKEN", "s3cret-value".as_ref()),
        ("SWITCHYARD_BUNDLE", "x".as_ref()),
    ]);
    assert!(!refused.status.success());
    let message = String::from_utf8_lossy(&refused.stderr).into_owned();
    assert!(
        message.contains("SWITCHYARD_ROUTER_TOKEN -> APMPR_ROUTER_TOKEN"),
        "{message}"
    );
    assert!(
        message.contains("SWITCHYARD_BUNDLE -> APMPR_BUNDLE"),
        "{message}"
    );
    // Names only. One of these variables is a router token.
    assert!(!message.contains("s3cret-value"), "{message}");
}

#[test]
fn the_binary_accepts_the_replacement_names() {
    assert!(
        run(vec![("APMPR_ROUTER_TOKEN", "s3cret-value".as_ref())])
            .status
            .success()
    );
}

/// An unrelated variable holding non-UTF-8 bytes is not this tool's business, and reading the
/// environment must not turn an ordinary command into a crash.
#[test]
fn an_unrelated_non_utf8_environment_value_is_not_this_tools_business() {
    let non_utf8 = OsStr::from_bytes(&b"\xff"[..]);
    let output = run(vec![("UNRELATED", non_utf8)]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
