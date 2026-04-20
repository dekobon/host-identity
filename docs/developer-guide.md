# Developer guide

This is the canonical reference for working on `host-identity`. It
covers the conventions, workflow, and tooling choices the project
runs on. If you're looking for a quick tour of *what* the crate does,
start with the [`README`](../README.md); this document is about *how*
we build it.

The guide is organised so you can read it top-to-bottom once, then
jump back to the relevant section when you need it.

## Contents

- [Getting started](#getting-started)
- [Workflow](#workflow)
- [Rust conventions](#rust-conventions)
- [Naming](#naming)
- [Testing](#testing)
- [Git and commits](#git-and-commits)
- [GitHub CLI](#github-cli)
- [Tooling choices](#tooling-choices)
- [Documentation](#documentation)
- [Changelog](#changelog)
- [Markdown](#markdown)
- [Bash scripts](#bash-scripts)
- [Lessons learned](#lessons-learned)
- [Adding an identity source](#adding-an-identity-source)
- [App-specific derivation](#app-specific-derivation)

## Getting started

```bash
git clone https://github.com/dekobon/host-identity
cd host-identity
cargo build
cargo test --all-features --workspace
```

MSRV is declared in the workspace `Cargo.toml` as `rust-version`.

Before pushing, the expected clean run is:

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --workspace
```

## Workflow

Open an issue before starting anything non-trivial. Design discussion
is cheaper than re-review of a large PR that went the wrong way.

1. Branch from `main`.
2. Keep commits small and reviewable. Each commit should ideally build
   and pass tests on its own.
3. Run fmt, clippy, and tests locally before pushing.
4. Open the PR against `main` and link the issue with `Fixes #NNN` in
   the body.
5. Add a changelog entry for user-visible changes (see
   [Changelog](#changelog)).

Reviews focus on correctness, API shape, and test coverage before
style. Criticism is welcome — point out mistakes, suggest better
approaches, cite standards. Be skeptical and concise.

## Rust conventions

### Formatting and lints

- `cargo fmt` for formatting, `cargo clippy` for linting.
- `unsafe_code = "forbid"` at the crate root — never write `unsafe`.
- Crate-level lints live in `Cargo.toml`. Don't override at the item
  level without justification.

### Error handling

- Avoid `unwrap()`, `expect()`, `assert!()`, and `panic!()` outside
  tests. Return `Result` / `Option` and propagate with `?`.
- In enumeration or discovery loops, a panic kills the remaining
  iterations. Return an error so the caller can skip and continue.
- Use `thiserror` for library error types; `anyhow` is fine in tests
  and binaries.

### Data modelling

- Prefer `enum` for state machines over boolean flags or loosely
  related fields.
- Use newtype wrappers to enforce domain invariants
  (`struct Port(u16)` instead of bare `u16`).
- Model invariants with types where possible (`NonZeroU32`,
  `Duration`, custom enums).
- Choose ownership deliberately per field: `&str` vs `String`, slices
  vs `Vec`, `Arc<T>` for shared ownership, `Cow<'a, T>` for flexible
  ownership.
- Prefer borrowing over cloning when the owned value isn't needed.

### Visibility

- Prefer `pub(crate)` over `pub`. Only expose what external callers
  actually need.
- Keep public APIs small and expressive; avoid leaking internal types.

### Code organisation

- Place `impl` blocks immediately below the type they implement.
- Group methods: constructors first, then getters, mutation methods,
  domain logic, helpers.
- Provide clear constructors (`new`, `with_capacity`, builder pattern)
  where appropriate.
- Implement `From` / `TryFrom` / `Display` / `Debug` to simplify
  conversions. Deriving `From` gives you `Into` for free.
- Use `derive` macros (`Debug`, `Clone`, `Serialize`, `Deserialize`)
  to reduce boilerplate.

### Public-API input validation

Guard public functions against degenerate inputs (empty strings,
zero-length slices) that would otherwise cause silent misbehaviour.
Prefer early `return None` / `return Err(...)` over letting degenerate
values flow through string operations like `split`, `splitn`,
`contains`, `find`. Note in particular that `str::splitn(n, "")`
splits on every character boundary — always guard against empty
patterns.

### Path-to-string conversion

- Never use `to_string_lossy()` for paths used as identifiers (map
  keys, JSON fields, error correlation). Use `to_str()` with explicit
  error handling.
- `path.display()` is fine for human-readable error messages and log
  output.
- `to_string_lossy()` is fine only for display formatting where a
  replaced codepoint isn't a correctness concern.

### Unreachable defensive code

If a code path is provably unreachable, use
`expect("invariant explanation")` rather than fallback logic. Don't
use `eprintln!` or logging in unreachable branches — that masks bugs.

### Build speed

- Use `cargo check` during rapid iteration instead of `cargo build`.
- Minimise unnecessary dependencies and feature flags. Prefer
  `default-features = false` when the default feature set isn't
  needed.

## Naming

Names should tell the truth about what the code does. Linters catch
mechanical violations; these rules catch semantic mismatches.

### Universal

- **One word per concept**: pick one verb and use it everywhere.
  Don't mix `fetch`/`get`/`retrieve` or `parse`/`from_str`/`decode`
  for the same operation.
- **Different words for different concepts**: if two things do
  different work, they need different names. Don't reuse `process`
  for both "validate input" and "transform output".
- **Boolean names are positive predicates**: `is_valid`,
  `has_children`, `can_retry` — never `not_disabled` or `no_error`.
  Double negation is a bug magnet.
- **No unexplained abbreviations**: domain-standard abbreviations
  (`fd`, `pid`, `url`, `uuid`, `dmi`) are fine. Project-specific or
  ad-hoc abbreviations need a comment or a full name.
- **Name length matches scope**: single-letter names are fine in
  tight closures. Public APIs, struct fields, and module names
  should be descriptive.
- **Plural names for collections**: `errors: Vec<Error>`, not
  `error: Vec<Error>`.

### Rust-specific

Conversion method prefixes must match semantics ([C-CONV](https://rust-lang.github.io/api-guidelines/naming.html#c-conv)):

- `as_` — free, borrowed view (`as_str()` returns `&str`)
- `to_` — expensive conversion, new allocation (`to_string()`)
- `into_` — consuming, takes ownership of `self` (`into_inner()`)
- `from_` — constructor from another type (`from_bytes()`)

The signature must match the prefix: `as_` borrows, `into_` consumes.

Other Rust conventions:

- Getters omit `get_` prefix: `fn name(&self) -> &str`
  ([C-GETTER](https://rust-lang.github.io/api-guidelines/naming.html#c-getter)).
- `is_` / `has_` methods return `bool`.
- Error type word order follows stdlib: `ParseError`, `ResolveError`.
- Type names match their semantic role. A field `count: String` or
  `name: Vec<u8>` is a red flag.

## Testing

### Build before test

Always rebuild (`cargo build` or `cargo check`) before running tests.
Never test against a stale binary.

### Discipline

- Never rewrite an entire test file to add or fix tests. Modify only
  the specific tests or functions that need changing.
- Verify previously passing tests still pass before committing.
- When fixing a bug, add a regression test that would catch the exact
  bug if reintroduced. Otherwise the fix is one commit away from
  being undone.
- Test names must describe the scenario and expected outcome. Not
  `test_resolve_1` — `resolve_returns_error_when_chain_is_empty`.

### Assertion strength

Tests that assert `is_ok()` or `!is_empty()` without checking the
actual value are weak. Assert the specific value, shape, or error
variant you expect. `unwrap()` / `expect()` are fine in tests — they
turn an unexpected `None`/`Err` into a useful failure.

### No incidental coupling

Before encoding any property into test code, verify that the *system
under test* actually depends on it. If the code doesn't branch on a
property, the tests must not couple to it. Prefer runtime discovery
(scanning, globbing) over constructing exact values from environmental
or structural assumptions.

Common traps:

- **Host environment** (`cfg(target_arch)`, `std::env::consts::ARCH`):
  only use if the code under test is architecture- or OS-specific.
- **Filename structure**: don't parse and reconstruct every segment of
  a naming convention — couple only to the segments the code needs.
- **Directory layout**: don't hardcode paths that reflect
  organisational choices irrelevant to the logic being tested.

### Environment

- Use `tempfile::NamedTempFile` / `TempDir` for filesystem fixtures,
  so tests clean up after themselves and don't depend on paths
  outside the crate.
- Don't set or read global environment variables across tests unless
  the test is marked `#[serial]` (via the `serial_test` crate). Env
  state leaks across parallel test threads.

## Git and commits

### Conventional commit format

- **Format**: `<type>(<scope>): <subject>`
- **Types**: `feat`, `fix`, `docs`, `style`, `refactor`, `test`,
  `chore`, `perf`
- **Subject**: max 50 chars, imperative mood, no period.
- **Body**: 72-char lines for complex changes, explaining what and
  why.

Keep commits atomic. Don't add `Co-Authored-By` lines.

Scope is typically one of: `sources`, `resolver`, `wrap`, `error`,
`docs`, `ci`, or the module name you touched.

### Closing issues

When a commit resolves a GitHub issue, add `Fixes #NNN` in the commit
**body** (not the subject line) — that auto-closes the issue on push.

```text
fix(sources): reject uninitialized sentinel on all machine-id sources

The DBus fallback was hashing the systemd sentinel instead of
rejecting it, producing colliding IDs on hosts caught in early boot.

Fixes #42
```

- Use `Fixes #N` in the body, not `(#N)` in the subject (that only
  creates a link).
- Multiple issues: one `Fixes #N` line per issue.
- GitHub recognises `Fixes`, `Closes`, `Resolves` (case-insensitive).

### Amending and squashing

Amending unpushed commits is fine and often preferred to keep history
clean. Use `git commit --amend` or
`git reset --soft HEAD~N && git commit` to squash local fixups into
their parent.

Only avoid amending commits that have already been pushed — that
requires a force-push and rewrites shared history.

## GitHub CLI

Use `gh` for all GitHub-related tasks: issues, pull requests, checks,
releases. If given a GitHub URL, use `gh` to fetch it.

For complex `gh` invocations with multi-line Markdown bodies, write
the content to a temp file and pass it with `--body-file`. Shell
quoting of backticks, `$variables`, and `"quotes"` is error-prone.

```bash
cat > /tmp/issue-body.md <<'EOF'
Content with $variables, `backticks`, and "quotes"
EOF
gh issue create --title "Title" --label "bug" \
  --body-file /tmp/issue-body.md
```

Issue hygiene:

- Only close an issue when all items in it are resolved.
- When updating an issue, update the body *and* add a comment — the
  body reflects current state, the comment records what changed.
- Never edit other people's comments.

## Tooling choices

### Searching and finding files

Prefer `rg` (ripgrep) over `grep`, and `fd` over `find`. They're
faster, respect `.gitignore` by default, and have better defaults.
The legacy `grep` and `find` commands are rarely the right choice —
reach for them only if `rg`/`fd` genuinely can't express the query.

### When Bash is appropriate

- Running `cargo check`, `cargo test`, `cargo clippy`, `cargo fmt`.
- Invoking `gh` for GitHub operations.
- Running `git` commands.
- `rg` / `fd` for searches beyond a simple string match.

## Documentation

### Audience

Keep these clearly separated:

- [`README.md`](../README.md) — the problem the crate solves, the
  default API, and enough detail to make a decision about using it.
- [`CHANGELOG.md`](../crates/host-identity/CHANGELOG.md) — what
  changed between releases.
- Doc comments (`///`, `//!`) — how to use a specific type, function,
  or module.
- This guide — how contributors work in this repository.

A README is not a changelog; a doc comment is not a spec.

### Rustdoc

- Use `///` on public structs, enums, traits, and non-obvious
  methods. The crate warns on `missing_docs`; don't silence it
  without reason.
- Use `//!` for module-level documentation explaining design intent
  or architecture.
- Include examples in doc comments where they clarify non-obvious
  usage.

### No stale counts

Never hardcode specific counts in documentation, comments, or
specifications. They go stale immediately.

- **Bad**: "25 tests passing", "6 sources", "71% pass rate".
- **Good**: "all tests passing", "every platform source", "majority
  of tests pass".
- Use approximate language when scale matters: "hundreds of
  platforms", "dozens of tests".
- **Exceptions**: `CHANGELOG.md` entries (point-in-time snapshots)
  and code (compiler- or test-verified).

## Changelog

Record significant features, fixes, and behavioural changes in
[`CHANGELOG.md`](../crates/host-identity/CHANGELOG.md) following
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format.

Sections, in order:

- `Added` — new features or public APIs
- `Changed` — behaviour changes in existing features
- `Deprecated` — soon-to-be-removed features
- `Removed` — removed features
- `Fixed` — bug fixes
- `Security` — vulnerability fixes

One line per change. Keep entries factual, terse, past tense:

```markdown
### Added

- `Resolver::with_wrap` lets callers choose a UUID wrap strategy.
- `FnSource` wraps a user closure as a `Source`.

### Fixed

- `MachineIdFile` now rejects the `uninitialized` sentinel before
  wrapping.
```

When to add:

- Public API additions, removals, or signature changes → always.
- Bug fixes with user-visible effect → yes.
- Refactors, documentation-only updates, CI changes → no (the commit
  message is enough).

## Markdown

If you add [markdownlint-cli2](https://github.com/DavidAnson/markdownlint-cli2)
or similar, run it after editing Markdown files. Until then:

- Line length under ~100 characters.
- ATX-style headings (`# Heading`).
- Fenced code blocks with language identifiers (`rust`, `bash`,
  `toml`).
- No trailing whitespace.
- One blank line between headings and the content above them.
- Inline code for file paths and identifiers
  (`` `Resolver::resolve` ``), not bold or italics.

## Bash scripts

Only relevant when the task genuinely needs Bash — the first choice
is always `cargo` or `gh` directly.

- Run `shellcheck` on any script you add.
- `set -euo pipefail` at the top of every script.
- Double-quote variable expansions; use `$(...)` for command
  substitution.
- Uppercase variable names with underscores (`FILE_PATH`).
- Use functions for reusable logic; avoid mutable global state.
- Lines under 100 characters; `getopts` for option parsing.

### `fd` / `fdfind` portability

The `fd` binary is named `fdfind` on Debian and Ubuntu. Scripts must
detect the correct name at startup:

```bash
FD=$(command -v fd 2>/dev/null || command -v fdfind 2>/dev/null || true)
if [[ -z "$FD" ]]; then
    echo "error: fd (or fdfind) not found." >&2
    exit 1
fi
```

Use `"$FD"` (quoted) instead of bare `fd` throughout.

## Release and packaging

Releases are cut by pushing a `v*` git tag. The
[`.github/workflows/release.yml`](../.github/workflows/release.yml)
pipeline handles everything downstream: binary builds,
distro packages, smoke tests, signing, SBOMs, SLSA provenance, the
GitHub Release, and updates to the Homebrew tap and Scoop bucket.
See
[`packaging/README.md`](../packaging/README.md) for the template
files each package format consumes.

For the full walkthrough — prerequisites, pre-release checklist,
rehearsing, monitoring, post-release verification, and recovery —
see [`docs/cutting-a-release.md`](cutting-a-release.md). The summary
below is the quick reference.

### Cutting a release

1. Land every change for the release on `main`.
2. Update the workspace `version` in `Cargo.toml`.
3. Add a `## [x.y.z]` section to
   `crates/host-identity/CHANGELOG.md` — the release workflow
   extracts this verbatim as the GitHub Release body and fails if
   it is missing.
4. Run `cargo xtask` and commit any `man/` diff.
5. `git tag -a vX.Y.Z -m "vX.Y.Z"` and `git push --tags`.

A pre-release (`vX.Y.Z-rc.1`) skips tap/bucket updates but
still publishes signed artefacts to the GitHub Release.

### Rehearsing a release

Use the `workflow_dispatch` trigger on the `Release` workflow with
a test tag like `v0.0.0-test1` (the pre-release gate prevents the
rehearsal from touching external repositories). Re-running the
workflow on the same tag is idempotent — `softprops/action-gh-release`
overwrites existing assets and tap/bucket pushes are no-ops when
the rendered files are unchanged.

### Secrets

The workflow expects these repository secrets:

| Secret                      | Used by                      | Scope                                           |
| --------------------------- | ---------------------------- | ----------------------------------------------- |
| `MINISIGN_SECRET_KEY`       | `sign-attest` job            | minisign private key (ASCII-armoured)           |
| `MINISIGN_PASSWORD`         | `sign-attest` job            | password for the minisign key                   |
| `ALPINE_ABUILD_KEY_PRIV`    | `package-apk` jobs           | abuild RSA private key                          |
| `ALPINE_ABUILD_KEY_PUB`     | `package-apk` jobs           | matching public key                             |
| `HOMEBREW_TAP_TOKEN`        | `publish` job                | fine-grained PAT scoped to `homebrew-host-identity` |
| `SCOOP_BUCKET_TOKEN`        | `publish` job                | fine-grained PAT scoped to `scoop-bucket`       |

Rotate the minisign keypair by generating a new one
(`minisign -G`), committing the new public key to `minisign.pub`,
and replacing the `MINISIGN_SECRET_KEY` / `MINISIGN_PASSWORD`
secrets. The next release self-documents the rotation via the new
`SHA256SUMS.minisig`.

### Deferred (known gaps)

- macOS code signing + notarization (Gatekeeper quarantines
  direct-download tarballs; Homebrew is unaffected).
- Windows code signing (SmartScreen friction for zip + Scoop users).
- MSI / Chocolatey / homebrew-core submission — add if requested.
- `aarch64-unknown-freebsd` — Rust tier 3; no pre-built artefact.

## Lessons learned

Hard-won project lessons live in
[`docs/lessons-learned.md`](lessons-learned.md) (create the file on
first entry). Keep the list small and actionable — only document
lessons that are genuinely hard (cost real debugging time or caused
real bugs) and important (likely to recur). Err on the side of not
adding entries. This is not a changelog or a diary.

Shape of an entry:

- A short title describing the failure mode, not the fix.
- The observed bug or surprising behaviour.
- The underlying cause.
- The prevention rule that future code should follow.

Example:

> **`/etc/machine-id` can contain the literal string `uninitialized`**
>
> During early boot, systemd writes the sentinel `uninitialized` and
> overmounts it once first-boot provisioning completes. Naive readers
> hash it like any other value, emitting the same UUID on every host
> caught in that window.
>
> *Rule*: any machine-id-shaped source must normalize and reject the
> sentinel before wrapping. See `sources/util.rs::normalize`.

## Adding an identity source

See [`adding-an-identity-source.md`](adding-an-identity-source.md) for
the end-to-end checklist: categorising the source, the file-by-file
changes, per-category recipes (local, plaintext cloud, bespoke cloud),
required tests, documentation updates, and PR expectations.

## App-specific derivation

`AppSpecific<S>` is a wrapper source: it takes any inner `Source`,
HMAC-SHA256s the inner probe value under a caller-supplied `app_id`,
truncates to 16 bytes with the UUID v4 version and variant-10 bits
forced, and emits the result as a hyphenated UUID string. This is the
same construction systemd uses for
`sd_id128_get_machine_app_specific()`, generalised to every source
the crate abstracts.

**Why you want it.** Handing a raw machine-id (or DMI UUID, or Windows
MachineGuid, or cloud instance ID) to telemetry is exactly what
[`machine-id(5)`](https://www.freedesktop.org/software/systemd/man/machine-id.html)
warns against: every app that reads the raw key gets the same
cross-correlatable ID for the host. `AppSpecific` gives each app a
distinct, stable identifier per host and prevents the raw key from
ever leaving the process.

**Shape.** Output is a UUID string, matching the other UUID-native
sources (`DmiProductUuid`, `IoPlatformUuid`, `WindowsMachineGuid`,
`KenvSmbios`). Consequences:

- `Wrap::Passthrough` round-trips the probe unchanged.
- Default `Wrap::UuidV5Namespaced` re-hashes the UUID string for
  crate-namespace separation the same way it does for the UUID-native
  sources — not double-hashing an already-hashed 256-bit value.

**Scope.** `AppSpecific<S>` inherits `S`'s identity scope. Wrapping a
per-host source (`MachineIdFile`) yields a per-host-per-app ID;
wrapping a per-instance source (`AwsImds`) yields a per-instance-per-app
ID; wrapping a per-pod source (`KubernetesPodUid`) yields a
per-pod-per-app ID. See
[`docs/algorithm.md` → "Identity scope"](algorithm.md#identity-scope-what-host-means-per-source)
for the scope rules.

**systemd byte-compat caveat.** `systemd-id128 machine-id
--app-specific=<X>` uses the **parsed 16 raw bytes** of
`/etc/machine-id` as the HMAC key. The built-in `MachineIdFile`
source emits the 32 hex ASCII characters instead, so
`AppSpecific<MachineIdFile>` HMACs with a 32-byte ASCII key —
different bytes, same UUID shape. Exact byte-compat requires wrapping
a custom source (e.g. a `FnSource` that parses the hex into 16 raw
bytes) and passing a 16-byte UUID-derived `app_id`. Rust callers that
don't need systemd interop can pass arbitrary `&[u8]` for `app_id`;
the privacy property holds regardless.

**Privacy caveats.**

- The inner raw value is the HMAC key; treat it as sensitive. The
  `hmac` crate holds its own internal copy of the key which this
  crate cannot zeroize. The `app_id` buffer is zeroized on drop as a
  best-effort.
- Wrapping a source whose raw value is already public (cloud instance
  IDs in consoles, Kubernetes pod UIDs readable via the API server)
  adds no privacy — the input isn't secret in the first place.
- The derived UUID is an identifier, **not** key material. Do not use
  it as a cryptographic key.

**Not in default chains.** Derivation requires an `app_id`, a caller
concern. `default_chain` / `network_default_chain` are unchanged;
callers opt in explicitly.
