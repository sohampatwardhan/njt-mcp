//! Process exit behavior for missing or malformed configuration.
#![allow(unknown_lints)]
#![deny(clippy::missing_docs_in_private_items)]

use std::process::Command;

/// Binary invocation with a cleared environment.
///
/// # Returns
///
/// A command ready to receive `NJT_*` variables.
///
/// # Panics
///
/// This function does not panic.
///
/// # Examples
///
/// The tests below add or omit credentials and assert on stderr.
fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_njt-mcp"));
    command
        .arg("stdio")
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default());
    command
}

/// Missing credentials exit 1 and name both forms.
#[test]
fn missing_credentials_exit_nonzero() {
    let output = command().output().expect("run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("NJT_TOKEN"), "{stderr}");
    assert!(stderr.contains("NJT_USERNAME"), "{stderr}");
    assert!(output.stdout.is_empty());
}

/// A bad optional value exits 1 without echoing the token.
#[test]
fn malformed_env_does_not_echo_the_token() {
    let output = command()
        .env("NJT_TOKEN", "super-secret-token")
        .env("NJT_ENV", "nope")
        .output()
        .expect("run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("NJT_ENV"), "{stderr}");
    assert!(!stderr.contains("super-secret-token"), "{stderr}");
}
