#shellcheck shell=sh
#shellcheck disable=SC2154 # HOST_IDENTITY_BIN is exported by spec_helper.sh.
# Shell-level precedence and edge-case coverage for the HOST_IDENTITY
# and HOST_IDENTITY_FILE environment variables. Complements — does not
# duplicate — `crates/host-identity-cli/tests/host_identity_file_env.rs`.

Describe 'HOST_IDENTITY / HOST_IDENTITY_FILE overrides'
  A=11111111-2222-3333-4444-555555555555
  B=22222222-3333-4444-5555-666666666666

  setup() {
    clean_host_identity_env
    TMPDIR_=$(fresh_tmpdir)
  }
  cleanup() { rm -rf "${TMPDIR_}"; }
  BeforeEach 'setup'
  AfterEach 'cleanup'

  Describe 'HOST_IDENTITY inline override'
    It 'prints the pinned UUID byte-for-byte with --wrap passthrough'
      export HOST_IDENTITY="${A}"
      When call host_identity resolve --wrap passthrough
      The status should equal 0
      The stderr should be blank
      The stdout should equal "${A}"
    End

    It 'survives an env -i invocation (no inherited PATH / HOME / LANG)'
      envi_resolve() {
        env -i HOST_IDENTITY="${A}" "${HOST_IDENTITY_BIN}" resolve --wrap passthrough
      }
      When call envi_resolve
      The status should equal 0
      The stdout should equal "${A}"
    End
  End

  Describe 'HOST_IDENTITY_FILE'
    It 'reads a UUID with a trailing newline'
      printf '%s\n' "${A}" > "${TMPDIR_}/id"
      export HOST_IDENTITY_FILE="${TMPDIR_}/id"
      When call host_identity resolve --wrap passthrough
      The status should equal 0
      The stdout should equal "${A}"
    End

    It 'reads a UUID with no trailing newline'
      printf '%s' "${A}" > "${TMPDIR_}/id"
      export HOST_IDENTITY_FILE="${TMPDIR_}/id"
      When call host_identity resolve --wrap passthrough
      The status should equal 0
      The stdout should equal "${A}"
    End

    It 'reads a UUID with CRLF line terminator'
      printf '%s\r\n' "${A}" > "${TMPDIR_}/id"
      export HOST_IDENTITY_FILE="${TMPDIR_}/id"
      When call host_identity resolve --wrap passthrough
      The status should equal 0
      The stdout should equal "${A}"
    End

    It 'reads a UUID with leading and trailing whitespace'
      printf '   %s   \n' "${A}" > "${TMPDIR_}/id"
      export HOST_IDENTITY_FILE="${TMPDIR_}/id"
      When call host_identity resolve --wrap passthrough
      The status should equal 0
      The stdout should equal "${A}"
    End

    It 'outranks HOST_IDENTITY when both are set'
      printf '%s\n' "${A}" > "${TMPDIR_}/id"
      export HOST_IDENTITY_FILE="${TMPDIR_}/id"
      export HOST_IDENTITY="${B}"
      When call host_identity resolve --wrap passthrough
      The status should equal 0
      The stdout should equal "${A}"
    End

    It 'falls through to HOST_IDENTITY when set empty'
      export HOST_IDENTITY_FILE=""
      export HOST_IDENTITY="${B}"
      When call host_identity resolve --wrap passthrough
      The status should equal 0
      The stdout should equal "${B}"
    End

    It 'falls through when the path does not exist'
      export HOST_IDENTITY_FILE="${TMPDIR_}/never-created"
      export HOST_IDENTITY="${B}"
      When call host_identity resolve --wrap passthrough
      The status should equal 0
      The stdout should equal "${B}"
    End
  End
End
