#shellcheck shell=sh
# Shell-level coverage for --app-id. Complements the integration tests
# in `crates/host-identity-cli/tests/app_specific.rs` by pinning the
# shell-surface contract (exit code, stderr message, JSON provenance).

Describe '--app-id wraps every source with AppSpecific'
  A=11111111-2222-3333-4444-555555555555

  setup() { clean_host_identity_env; }
  BeforeEach 'setup'

  It 'is deterministic: same HOST_IDENTITY + same --app-id => same UUID'
    export HOST_IDENTITY="$A"
    first=$(host_identity resolve --app-id com.example.a)
    second=$(host_identity resolve --app-id com.example.a)
    When call test "$first" = "$second"
    The status should equal 0
    The value "$first" should match pattern "$UUID_SHAPE"
  End

  It 'is uncorrelatable: different --app-id => different UUIDs'
    export HOST_IDENTITY="$A"
    a=$(host_identity resolve --app-id com.example.a)
    b=$(host_identity resolve --app-id com.example.b)
    When call test "$a" != "$b"
    The status should equal 0
    The value "$a" should match pattern "$UUID_SHAPE"
    The value "$b" should match pattern "$UUID_SHAPE"
  End

  It 'prefixes the source label with app-specific: in JSON output'
    export HOST_IDENTITY="$A"
    When call host_identity resolve --app-id com.example.a --format json
    The status should equal 0
    The stdout should include '"source": "app-specific:env-override"'
  End

  It 'round-trips the AppSpecific UUID under --wrap passthrough'
    export HOST_IDENTITY="$A"
    When call host_identity resolve --app-id com.example.a --wrap passthrough
    The status should equal 0
    The stdout should match pattern "$UUID_SHAPE"
  End

  It 'rejects an empty --app-id with EXIT_USAGE (2)'
    export HOST_IDENTITY="$A"
    When call host_identity resolve --app-id ''
    The status should equal 2
    The stderr should include 'must not be empty'
    The stdout should be blank
  End
End
