//! End-to-end coverage for the `HOST_IDENTITY_FILE` environment
//! override. Regression test for issue #8: the help text advertised
//! the override but the CLI never read the variable.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The pinned UUID written into the override file. Any well-formed v4
/// UUID works; we pick a literal constant so assertions can compare
/// strings directly.
const PINNED_UUID: &str = "11111111-2222-3333-4444-555555555555";

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_host-identity"))
}

/// Allocate a unique scratch path under the system temp dir. Avoids a
/// `tempfile` dev-dep for a single-file test, and the counter keeps
/// concurrent `cargo test` runs from colliding on the same filename.
fn unique_scratch(label: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "host-identity-cli-{label}-{}-{nanos}-{n}",
        std::process::id(),
    ))
}

fn run_with_file_env(
    path_value: Option<&std::path::Path>,
    extra_args: &[&str],
) -> (std::process::Output, Vec<String>) {
    let mut cmd = Command::new(bin());
    cmd.arg("resolve")
        .arg("--wrap")
        .arg("passthrough")
        .args(extra_args)
        .env_remove("HOST_IDENTITY")
        .env_remove("HOST_IDENTITY_FILE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(p) = path_value {
        cmd.env("HOST_IDENTITY_FILE", p);
    }
    let out = cmd.output().expect("spawn host-identity");
    let stdout_lines = String::from_utf8(out.stdout.clone())
        .expect("stdout is utf-8")
        .lines()
        .map(str::to_owned)
        .collect();
    (out, stdout_lines)
}

#[test]
fn host_identity_file_pins_identity_with_default_chain() {
    let path = unique_scratch("default");
    fs::write(&path, format!("{PINNED_UUID}\n")).expect("write override file");

    let (out, lines) = run_with_file_env(Some(&path), &[]);
    let _ = fs::remove_file(&path);

    assert!(
        out.status.success(),
        "CLI must succeed; stderr={:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(lines.first().map(String::as_str), Some(PINNED_UUID));
}

#[test]
fn host_identity_file_pins_identity_with_explicit_sources() {
    // `--sources machine-id` alone would normally read /etc/machine-id;
    // the override must still apply, proving we prepend it even when
    // the user chose an explicit chain.
    let path = unique_scratch("explicit");
    fs::write(&path, format!("{PINNED_UUID}\n")).expect("write override file");

    let (out, lines) = run_with_file_env(Some(&path), &["--sources", "machine-id"]);
    let _ = fs::remove_file(&path);

    assert!(
        out.status.success(),
        "CLI must succeed; stderr={:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(lines.first().map(String::as_str), Some(PINNED_UUID));
}

#[test]
fn host_identity_file_outranks_host_identity_env() {
    // When both are set, HOST_IDENTITY_FILE must win — the documented
    // precedence in LONG_ABOUT. Without this check a future refactor
    // could silently reorder the two and no one would notice.
    let path = unique_scratch("precedence");
    fs::write(&path, format!("{PINNED_UUID}\n")).expect("write override file");

    let out = Command::new(bin())
        .arg("resolve")
        .arg("--wrap")
        .arg("passthrough")
        .env_remove("HOST_IDENTITY")
        .env("HOST_IDENTITY", "99999999-9999-9999-9999-999999999999")
        .env("HOST_IDENTITY_FILE", &path)
        .stdin(Stdio::null())
        .output()
        .expect("spawn host-identity");
    let _ = fs::remove_file(&path);

    assert!(out.status.success(), "CLI must succeed");
    let stdout = String::from_utf8(out.stdout).expect("utf-8 stdout");
    assert_eq!(stdout.lines().next(), Some(PINNED_UUID));
}

#[test]
fn empty_host_identity_file_falls_through_to_env_override() {
    // HOST_IDENTITY_FILE= (set, empty) must behave as unset: no empty
    // relative-path probe, chain continues to HOST_IDENTITY.
    let uuid = "22222222-3333-4444-5555-666666666666";
    let out = Command::new(bin())
        .arg("resolve")
        .arg("--wrap")
        .arg("passthrough")
        .env("HOST_IDENTITY_FILE", "")
        .env("HOST_IDENTITY", uuid)
        .stdin(Stdio::null())
        .output()
        .expect("spawn host-identity");

    assert!(
        out.status.success(),
        "CLI must succeed; stderr={:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf-8 stdout");
    assert_eq!(stdout.lines().next(), Some(uuid));
}

#[test]
fn missing_host_identity_file_path_is_skipped_not_fatal() {
    // A nonexistent path must not error — FileOverride returns Ok(None)
    // on NotFound, letting the chain fall through. Set HOST_IDENTITY
    // as a guaranteed follow-up so the test is deterministic on a host
    // without /etc/machine-id.
    let missing = unique_scratch("missing-on-purpose");
    assert!(!missing.exists());
    let uuid = "44444444-5555-6666-7777-888888888888";
    let out = Command::new(bin())
        .arg("resolve")
        .arg("--wrap")
        .arg("passthrough")
        .env("HOST_IDENTITY_FILE", &missing)
        .env("HOST_IDENTITY", uuid)
        .stdin(Stdio::null())
        .output()
        .expect("spawn host-identity");

    assert!(
        out.status.success(),
        "CLI must succeed; stderr={:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf-8 stdout");
    assert_eq!(stdout.lines().next(), Some(uuid));
}
