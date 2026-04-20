#shellcheck shell=sh
# Specs for `host-identity sources`. Only asserts a feature-agnostic
# subset of identifiers — the exact list varies with `container` and
# `network` feature flags.

Describe 'host-identity sources'
  BeforeEach 'clean_host_identity_env'

  sources_plain_has_all() {
    # First whitespace-delimited column per line is the identifier;
    # keeping the assertion independent of the description column and
    # padding width.
    # shellcheck disable=SC2312 # awk's exit is folded into the pipeline.
    list=$(host_identity sources | awk '{print $1}')
    for id in env-override file-override machine-id; do
      printf '%s\n' "${list}" | grep -qxF "${id}" || return 1
    done
  }

  sources_stderr_blank() {
    err=$(host_identity sources 2>&1 >/dev/null)
    [ -z "${err}" ]
  }

  sources_json_check() {
    host_identity sources --json \
      | jq -e '
          type == "array"
          and length > 0
          and all(.[]; (.id | type == "string") and (.description | type == "string"))
          and (([.[] | .id] | index("env-override")) != null)
          and (([.[] | .id] | index("file-override")) != null)
          and (([.[] | .id] | index("machine-id")) != null)
        ' >/dev/null
  }

  It 'lists env-override, file-override, and machine-id in plain output'
    When call sources_plain_has_all
    The status should equal 0
  End

  It 'writes nothing to stderr'
    When call sources_stderr_blank
    The status should equal 0
  End

  It 'emits valid JSON containing the feature-agnostic identifier set under --json'
    Skip if 'jq is not installed' jq_missing
    When call sources_json_check
    The status should equal 0
  End
End
