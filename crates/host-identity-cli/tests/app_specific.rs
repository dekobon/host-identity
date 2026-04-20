//! End-to-end coverage for the `--app-id` flag, which wraps every
//! source in the chain with `AppSpecific` and emits a per-app UUID.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const PINNED_UUID: &str = "11111111-2222-3333-4444-555555555555";

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_host-identity"))
}

fn resolve(env_uuid: &str, extra: &[&str]) -> std::process::Output {
    Command::new(bin())
        .arg("resolve")
        .args(extra)
        .env_remove("HOST_IDENTITY_FILE")
        .env("HOST_IDENTITY", env_uuid)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn host-identity")
}

fn stdout_first_line(out: &std::process::Output) -> &str {
    std::str::from_utf8(&out.stdout)
        .expect("stdout is utf-8")
        .lines()
        .next()
        .unwrap_or("")
}

fn assert_success(out: &std::process::Output) {
    assert!(
        out.status.success(),
        "CLI failed: status={:?} stderr={:?}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn app_id_is_deterministic_under_pinned_env_override() {
    let first = resolve(PINNED_UUID, &["--app-id", "com.example.a"]);
    let second = resolve(PINNED_UUID, &["--app-id", "com.example.a"]);
    assert_success(&first);
    assert_success(&second);
    let a = stdout_first_line(&first);
    let b = stdout_first_line(&second);
    assert!(!a.is_empty(), "empty stdout from first run");
    assert_eq!(a, b, "same env + same app-id must yield the same UUID");
}

#[test]
fn different_app_ids_produce_different_uuids() {
    let a = resolve(PINNED_UUID, &["--app-id", "com.example.a"]);
    let b = resolve(PINNED_UUID, &["--app-id", "com.example.b"]);
    assert_success(&a);
    assert_success(&b);
    let ua = stdout_first_line(&a);
    let ub = stdout_first_line(&b);
    assert_ne!(
        ua, ub,
        "different app-ids on the same host must be uncorrelatable",
    );
    // Both are well-formed UUIDs.
    for u in [&ua, &ub] {
        assert_eq!(u.len(), 36, "expected hyphenated UUID, got {u:?}");
        assert_eq!(u.chars().filter(|&c| c == '-').count(), 4);
    }
}

#[test]
fn empty_app_id_exits_usage() {
    let out = resolve(PINNED_UUID, &["--app-id", ""]);
    assert!(!out.status.success(), "empty --app-id must fail");
    assert_eq!(
        out.status.code(),
        Some(2),
        "empty --app-id must exit with EXIT_USAGE (2)",
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("must not be empty"),
        "stderr should mention the validation; got {stderr:?}",
    );
}

#[test]
fn passthrough_wrap_preserves_app_specific_uuid() {
    // AppSpecific emits a hyphenated UUID string, so --wrap passthrough
    // must round-trip it successfully rather than raising a Malformed
    // error. Comparing against --wrap v5 proves passthrough actually
    // runs a different wrap path — otherwise a silent fallback to v5
    // would still produce deterministic output and the test would pass.
    let passthrough = resolve(
        PINNED_UUID,
        &["--app-id", "com.example.a", "--wrap", "passthrough"],
    );
    let v5 = resolve(PINNED_UUID, &["--app-id", "com.example.a", "--wrap", "v5"]);
    assert_success(&passthrough);
    assert_success(&v5);
    let pt = stdout_first_line(&passthrough);
    let v5_out = stdout_first_line(&v5);
    assert!(!pt.is_empty(), "empty passthrough stdout");
    assert_ne!(
        pt, v5_out,
        "--wrap passthrough must produce a different UUID than --wrap v5; \
         equal output means passthrough silently re-hashed",
    );
}

fn unique_scratch(label: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "host-identity-cli-app-specific-{label}-{}-{nanos}-{n}",
        std::process::id(),
    ))
}

#[test]
fn app_id_wraps_host_identity_file_override() {
    // HOST_IDENTITY_FILE is prepended ahead of every other source; this
    // test pins that --app-id still wraps it, so the documented claim
    // ("every source is wrapped, including HOST_IDENTITY_FILE") cannot
    // silently regress if the wrap step is ever reordered.
    let path = unique_scratch("file-override");
    fs::write(&path, format!("{PINNED_UUID}\n")).expect("write override file");

    let out = Command::new(bin())
        .arg("resolve")
        .args(["--app-id", "com.example.a", "--format", "json"])
        .env_remove("HOST_IDENTITY")
        .env("HOST_IDENTITY_FILE", &path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn host-identity");
    let _ = fs::remove_file(&path);

    assert_success(&out);
    let stdout = String::from_utf8(out.stdout).expect("stdout is utf-8");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(
        parsed["wrap"].as_str(),
        Some("v5"),
        "default --wrap must be reflected in JSON envelope",
    );
    let host_id = &parsed["host_id"];
    let source = host_id["source"].as_str().expect("source is a string");
    assert_eq!(
        source, "app-specific:file-override",
        "HOST_IDENTITY_FILE must be wrapped with app-specific",
    );
    let uuid = host_id["uuid"].as_str().expect("uuid is a string");
    assert_ne!(
        uuid, PINNED_UUID,
        "raw file contents must not leak through to the output UUID",
    );
}

#[test]
fn app_id_source_label_is_prefixed_in_json_output() {
    let out = resolve(
        PINNED_UUID,
        &["--app-id", "com.example.a", "--format", "json"],
    );
    assert_success(&out);
    let stdout = String::from_utf8(out.stdout).expect("stdout is utf-8");
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let source = parsed["host_id"]["source"]
        .as_str()
        .expect("source is a string");
    assert!(
        source.starts_with("app-specific:"),
        "source label should be prefixed; got {source:?}",
    );
}
