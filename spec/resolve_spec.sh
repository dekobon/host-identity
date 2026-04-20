#shellcheck shell=sh
#shellcheck disable=SC2016
# Specs for `host-identity resolve` on its own — format, wrap, sources
# validation. Precedence and env-override edge cases live in
# env_override_spec.sh.

Describe 'host-identity resolve'
  BeforeEach 'clean_host_identity_env'

  It 'prints a single UUID line on a host with a resolvable source'
    When call host_identity resolve
    The status should equal 0
    The stderr should be blank
    The stdout should match pattern "$UUID_SHAPE"
    The lines of stdout should equal 1
  End

  It 'defaults to resolve when no subcommand is given'
    When call host_identity
    The status should equal 0
    The stderr should be blank
    The stdout should match pattern "$UUID_SHAPE"
    The lines of stdout should equal 1
  End

  It 'produces a single non-empty line for --format summary'
    When call host_identity resolve --format summary
    The status should equal 0
    The stderr should be blank
    The lines of stdout should equal 1
  End

  It 'rejects an unknown source identifier with a usage exit code'
    When call host_identity resolve --sources definitely-not-a-source
    The status should equal 2
    The stdout should be blank
    The stderr should include 'unknown source identifier'
  End

  It 'rejects --network-timeout-ms without --network'
    When call host_identity resolve --network-timeout-ms 500
    The status should equal 2
    The stdout should be blank
    The stderr should include 'requires `--network`'
  End

  It 'rejects an unknown --format value with a usage exit code'
    When call host_identity resolve --format not-a-format
    The status should equal 2
    The stdout should be blank
    The stderr should not be blank
  End
End
