#shellcheck shell=sh
# Specs for `host-identity audit`: exit codes, status taxonomy, and the
# BrokenPipe contract that `write_and_flush` in the CLI promises.

Describe 'host-identity audit'
  BeforeEach 'clean_host_identity_env'

  It 'succeeds and writes at least one outcome line on a resolvable host'
    When call host_identity audit
    The status should equal 0
    The stderr should be blank
    The output should not be blank
  End

  It 'does not die from SIGPIPE when stdout is closed early'
    # Regression for the BrokenPipe branch in write_and_flush. Without
    # the shim, the audit process is killed by SIGPIPE (exit 141) when
    # `head` closes its read end. POSIX sh reports only the pipeline's
    # tail status, so we smuggle audit's real exit status through a
    # file.
    audit_head() {
      rc_file=$(mktemp)
      { host_identity audit; printf '%s' "$?" > "$rc_file"; } \
        | head -n1 >/dev/null
      rc=$(cat "$rc_file")
      rm -f "$rc_file"
      return "$rc"
    }
    When call audit_head
    The status should equal 0
  End

  It 'exits 1 with a stderr diagnostic when every source skips'
    # `--sources env-override` plus an unset HOST_IDENTITY guarantees
    # the only source skips, so audit must surface the runtime exit code
    # documented in the lib module. stdout still carries the per-source
    # report — that's the whole point of audit — so we acknowledge it.
    When call host_identity audit --sources env-override
    The status should equal 1
    The stderr should include 'no source produced a host identity'
    The stdout should include '(skipped)'
  End

  It 'emits valid JSON with the documented status taxonomy under --format json'
    Skip if 'jq is not installed' jq_missing
    audit_json_check() {
      host_identity audit --format json \
        | jq -e 'type == "array" and length > 0
                 and all(.[]; .source | type == "string")
                 and all(.[]; .status | IN("found","skipped","errored"))' \
          >/dev/null
    }
    When call audit_json_check
    The status should equal 0
  End

  It 'rejects an unknown --format value with a usage exit code'
    When call host_identity audit --format not-a-format
    The status should equal 2
    The stdout should be blank
    The stderr should not be blank
  End
End
