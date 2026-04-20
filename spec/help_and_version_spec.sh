#shellcheck shell=sh
#shellcheck disable=SC2016
# Help-text and version contracts that packagers and shell callers
# scrape. Tightly scoped on purpose — we assert the strings the
# ecosystem depends on, not clap's full wording.

Describe 'help and version'
  BeforeEach 'clean_host_identity_env'

  It '--version prints `host-identity <semver>` and exits 0'
    When call host_identity --version
    The status should equal 0
    The stderr should be blank
    The stdout should match pattern "$VERSION_SHAPE"
  End

  It '--help mentions both HOST_IDENTITY and HOST_IDENTITY_FILE'
    When call host_identity --help
    The status should equal 0
    The stderr should be blank
    The stdout should include 'HOST_IDENTITY'
    The stdout should include 'HOST_IDENTITY_FILE'
  End

  It 'resolve --help documents --sources, --wrap, and --format'
    When call host_identity resolve --help
    The status should equal 0
    The stderr should be blank
    The stdout should include '--sources'
    The stdout should include '--wrap'
    The stdout should include '--format'
  End

  It 'sources --help documents --json'
    When call host_identity sources --help
    The status should equal 0
    The stderr should be blank
    The stdout should include '--json'
  End

  It 'exits 2 with a stderr diagnostic for an unknown subcommand'
    When call host_identity bogus-subcommand
    The status should equal 2
    The stdout should be blank
    The stderr should not be blank
  End

  It 'exits 2 for an unknown top-level flag'
    When call host_identity --definitely-not-a-flag
    The status should equal 2
    The stdout should be blank
    The stderr should not be blank
  End
End
