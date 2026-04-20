#shellcheck shell=sh
# Consolidated JSON-validity assertions across every subcommand that
# offers a machine-readable format. If any of these fail while the
# subcommand-specific specs pass, the regression is in the formatter
# (e.g. log bleed onto stdout), not the subcommand logic.

Describe 'machine-readable output is valid JSON'
  BeforeEach 'clean_host_identity_env'

  resolve_json_parse() { host_identity resolve --format json | jq -e . >/dev/null; }
  audit_json_parse()   { host_identity audit   --format json | jq -e . >/dev/null; }
  sources_json_parse() { host_identity sources --json         | jq -e . >/dev/null; }

  It 'resolve --format json parses via jq'
    Skip if 'jq is not installed' jq_missing
    When call resolve_json_parse
    The status should equal 0
    The stderr should be blank
  End

  It 'audit --format json parses via jq'
    Skip if 'jq is not installed' jq_missing
    When call audit_json_parse
    The status should equal 0
    The stderr should be blank
  End

  It 'sources --json parses via jq'
    Skip if 'jq is not installed' jq_missing
    When call sources_json_parse
    The status should equal 0
    The stderr should be blank
  End
End
