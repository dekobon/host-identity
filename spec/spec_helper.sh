#shellcheck shell=sh
# ShellSpec harness helpers for the `host-identity` CLI.
#
# Binary discovery order (first match wins):
#   1. $HOST_IDENTITY_BIN if set and executable.
#   2. $CARGO_TARGET_DIR/debug/host-identity if set and executable.
#   3. <repo>/target/debug/host-identity if executable.
#   4. `command -v host-identity` (installed binary; used in smoke contexts).
#
# Fails loud when nothing resolves — every spec depends on the binary, so
# surfacing a single clear error beats a cascade of "command not found".

# shellspec load hook. Called once before any example group runs.
# Spec files MUST NOT source this directly; shellspec loads it via
# `--require spec_helper` from `.shellspec`.
spec_helper_precheck() {
  if ! HOST_IDENTITY_BIN=$(resolve_host_identity_bin); then
    echo "spec_helper: could not locate the host-identity binary." >&2
    echo "  Tried \$HOST_IDENTITY_BIN, \$CARGO_TARGET_DIR/debug, ./target/debug," >&2
    echo "  and \$(command -v host-identity). Run \`cargo build -p host-identity-cli\`" >&2
    echo "  or set HOST_IDENTITY_BIN to an installed binary before running shellspec." >&2
    return 1
  fi
  export HOST_IDENTITY_BIN
}

resolve_host_identity_bin() {
  if [ -n "${HOST_IDENTITY_BIN:-}" ] && [ -x "$HOST_IDENTITY_BIN" ]; then
    printf '%s\n' "$HOST_IDENTITY_BIN"
    return 0
  fi
  if [ -n "${CARGO_TARGET_DIR:-}" ] && [ -x "$CARGO_TARGET_DIR/debug/host-identity" ]; then
    printf '%s\n' "$CARGO_TARGET_DIR/debug/host-identity"
    return 0
  fi
  # SHELLSPEC_PROJECT_ROOT is set by shellspec to the directory containing
  # `.shellspec`; fall back to the current directory for raw `shellspec`
  # invocations from the repo root.
  repo_root=${SHELLSPEC_PROJECT_ROOT:-$PWD}
  if [ -x "$repo_root/target/debug/host-identity" ]; then
    printf '%s\n' "$repo_root/target/debug/host-identity"
    return 0
  fi
  if installed=$(command -v host-identity 2>/dev/null) && [ -n "$installed" ]; then
    printf '%s\n' "$installed"
    return 0
  fi
  return 1
}

# Invoke the binary under test. Specs call this instead of spelling out
# the full variable every time.
host_identity() { "$HOST_IDENTITY_BIN" "$@"; }

# Clear the override env vars so default-chain specs cannot be skewed by
# a user shell that exports one of them. Call from BeforeEach.
clean_host_identity_env() {
  unset HOST_IDENTITY HOST_IDENTITY_FILE
}

have_jq() { command -v jq >/dev/null 2>&1; }
jq_missing() { ! have_jq; }

# Shellspec's `match pattern` uses shell-style globs. Centralise them
# so specs stay readable and the patterns are defined exactly once.
# Trailing * absorbs a trailing newline from captured stdout.
UUID_SHAPE='????????-????-????-????-????????????*'
VERSION_SHAPE='host-identity [0-9]*.[0-9]*.[0-9]**'
export UUID_SHAPE VERSION_SHAPE

# Create a per-example tempdir. Specs capture the path and remove it in
# AfterEach — mktemp -d is portable to busybox/alpine as well.
fresh_tmpdir() { mktemp -d "${TMPDIR:-/tmp}/host-identity-spec.XXXXXX"; }

spec_helper_precheck
