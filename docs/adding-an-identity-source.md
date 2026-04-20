# Adding an identity source

End-to-end guide for adding a new identity source to `host-identity`,
from choosing the category through opening the PR.

Read the linked sections of the
[developer guide](developer-guide.md) for the broader project
conventions. This document is the task-specific checklist.

## Contents

- [Before you start](#before-you-start)
- [Categorise the source](#categorise-the-source)
- [Decide whether it joins a default chain](#decide-whether-it-joins-a-default-chain)
- [File-by-file checklist](#file-by-file-checklist)
- [Recipe A — local file or command source](#recipe-a--local-file-or-command-source)
- [Recipe B — cloud metadata (plaintext `CloudEndpoint`)](#recipe-b--cloud-metadata-plaintext-cloudendpoint)
- [Recipe C — cloud metadata (bespoke, like AWS IMDSv2)](#recipe-c--cloud-metadata-bespoke-like-aws-imdsv2)
- [Testing](#testing)
- [Documentation updates](#documentation-updates)
- [Pre-PR checks](#pre-pr-checks)
- [Opening the PR](#opening-the-pr)
- [Appendix — files that list every source](#appendix--files-that-list-every-source)

## Before you start

1. **Open an issue first.** Identity sources ship in the public API;
   the name, feature flag, and chain position are all stable
   commitments. Settle those in the issue before writing code.
2. **Find an authoritative reference.** A vendor doc, man page, RFC,
   or kernel ABI page. If there isn't one, the source probably isn't
   stable enough to ship. Link it in the issue.
3. **Confirm it's a host identity.** The source must yield a value
   that's stable for the lifetime of the host/instance/pod and distinct
   from its neighbours. A value that changes per-boot or collides
   across hosts is not an identity source — reject it at the issue
   stage.
4. **Pin down the identity scope.** Every source answers one of
   these questions, and the answer decides chain position:
   - *Per-pod* — Kubernetes pod UID, downward-API projected `uid`.
   - *Per-container* — OCI runtime container ID.
   - *Per-instance* — cloud metadata (`AwsImds`, etc.), SMBIOS,
     `IOPlatformUUID`.
   - *Per-host-OS* — `/etc/machine-id`, `MachineGuid`, `hostid`.
   - *Per-namespace* — Kubernetes service account.
   - *Caller-scoped* — env/file override, caller closure.

   See [`docs/algorithm.md` → "Identity scope"](algorithm.md#identity-scope-what-host-means-per-source)
   for the rules. Host-scope sources return the host's identity even
   when the caller is in a container, so they must sit **below**
   every container-scope and pod-scope source in the default chain.
   A new source that can't be placed without disturbing that
   ordering doesn't belong in a default chain — ship it as a
   constructible type that callers `push` explicitly.

## Categorise the source

The category decides which recipe you follow and which files you
touch.

| Category              | Feature gating                          | Recipe        |
| --------------------- | --------------------------------------- | ------------- |
| Platform-native (OS)  | `#[cfg(target_os = "…")]` + stubs       | [A](#recipe-a--local-file-or-command-source) |
| Optional local        | New Cargo feature, no `http` dep        | [A](#recipe-a--local-file-or-command-source) |
| Cloud metadata, plain | New Cargo feature inheriting `_transport` | [B](#recipe-b--cloud-metadata-plaintext-cloudendpoint) |
| Cloud metadata, bespoke | New Cargo feature inheriting `_transport` | [C](#recipe-c--cloud-metadata-bespoke-like-aws-imdsv2) |
| Kubernetes-adjacent   | Extend the `k8s` feature                | [A](#recipe-a--local-file-or-command-source) |
| Wrapper over `Source` | No feature (generic wrapper)            | see `AppSpecific` below                      |

A **wrapper source** composes with another `Source` rather than
probing a new identifier. `AppSpecific<S>` is the canonical example:
it HMAC-derives the inner source's probe value and emits a UUID
string so `Wrap::Passthrough` and `Wrap::UuidV5Namespaced` behave the
same as for the other UUID-native sources. Rule of thumb for wrapper
sources: **emit the same probe shape the existing UUID-native
sources emit.** A novel shape (e.g. a 64-char hex hash) breaks
`Wrap::Passthrough` and double-hashes under the default wrap.

If your source is a plaintext `GET` against a link-local metadata
endpoint and returns the identifier directly in the body, you want
recipe B. Only fall back to recipe C if the provider requires
something `CloudMetadata<E, T>` can't express (token dances, JSON
decoding, multiple round-trips).

## Decide whether it joins a default chain

Two chains live in `crates/host-identity/src/sources/mod.rs`:

- `default_chain()` — local only, no network. Platform-native sources
  for the current OS go here.
- `network_default_chain(transport)` — adds cloud and k8s sources.
  Ordered per [`docs/algorithm.md`](algorithm.md#ordering-principle):
  per-pod → per-container → per-instance → per-host software state →
  coarse fallback.

A source joins a chain only when it's broadly useful on hosts where
its feature is enabled. Niche or operator-pinned sources (file
override, downward-API projected file) ship as constructible types
but stay out of both defaults. When in doubt, leave it out — callers
can `.push(...)` it.

If your source joins `network_default_chain`, state its intended
position in the issue. Reordering the chain is a behavioural change
that needs a changelog entry.

## File-by-file checklist

Every new source touches these files. The appendix has a grep recipe
for finding every enumeration site if this list drifts.

**Library crate** (`crates/host-identity/`):

- `src/sources/<name>.rs` — the new source module (create).
- `src/sources/mod.rs` — module declaration + re-export + optional
  chain insertion.
- `src/source.rs` — new `SourceKind` variant and `source_kind_ids!`
  row.
- `src/ids.rs` — new `source_ids` constant, `local_source_from_id`
  arm (and `source_from_id_with_transport` arm for cloud), and
  round-trip test entry in `source_kind_from_id_round_trips_every_builtin`.
- `src/lib.rs` — feature mention in the cloud-metadata table / module
  intro when applicable.
- `Cargo.toml` — new feature flag (if the source is feature-gated).
- `CHANGELOG.md` — `### Added` entry under `## [Unreleased]`.
- `README.md` — feature row under `## Features`, source row under the
  cloud-provider table when applicable.

**CLI crate** (`crates/host-identity-cli/`):

- `src/lib.rs` — add the new identifier constant to
  `available_source_ids()` under the right `#[cfg(feature = …)]`.
- `Cargo.toml` — extend the `network` (or other) feature to pull in
  the new library feature when the CLI should ship it.

**Top-level docs** (`docs/`, repo root):

- `docs/algorithm.md` — source entry in the default-chain listing,
  per-source probe behaviour block, authoritative-reference row.
- `README.md` — any top-level feature or provider table that calls
  out this source explicitly.
- `docs/lessons-learned.md` — only if the work exposed a
  non-obvious, likely-to-recur trap. Err on the side of leaving it
  alone.

The appendix lists the grep patterns that find every enumeration
site, in case the list above has fallen out of sync.

## Recipe A — local file or command source

Use for `/etc/machine-id`-style files, sysctls, registry reads, or
command-output sources.

1. **Pick a name and identifier.**
   - Type name: `PascalCase` ending in a descriptive noun
     (`MachineIdFile`, `KenvSmbios`, `WindowsMachineGuid`).
   - Identifier string: short, lowercase, kebab-case
     (`machine-id`, `kenv-smbios`, `windows-machine-guid`). This is
     the string operators type in config; don't change it later.
   - `SourceKind` variant matches the type name minus any suffix
     that doesn't survive translation (`MachineIdFile` →
     `SourceKind::MachineId`).

2. **Create `src/sources/<name>.rs`** with a module-level `//!`
   doc comment that cites the authoritative reference. Implement
   `Source` for the new type. Follow the patterns in
   `src/sources/linux.rs` (file-backed) or
   `src/sources/freebsd.rs` (command-backed).

   Required probe behaviours:

   - `Ok(None)` when the input is absent: file not found, command
     missing, wrong OS, feature disabled.
   - `Ok(None)` when the input is present but empty or whitespace-only
     (via `sources::util::normalize`).
   - The `uninitialized` sentinel — **never `Ok(Some(...))`**. The
     exact response depends on the source category, because callers
     interpret the two differently:
     - Machine-id-shaped sources that read a **standard** path
       (`MachineIdFile`, `DbusMachineIdFile`, any future source that
       reads a well-known file contract) return
       `Err(Error::Uninitialized { path })`. The caller wanted a
       real machine-id and didn't get one; that's diagnostic.
     - Caller-pinned sources that read **operator-supplied** input
       (`EnvOverride`, `FileOverride`) return `Ok(None)` so the
       resolver falls through. The sentinel showing up in
       caller-controlled input is recoverable — the operator can
       just set a different value.

     Use `classify` from `sources::util` to distinguish the sentinel
     from an empty read in either path. See
     [`lessons-learned.md`](lessons-learned.md) for the `/etc/machine-id`
     rationale that drives this split.
   - `Err(Error::Io { path, source })` for I/O failures other than
     `NotFound` / `PermissionDenied`, which degrade to `Ok(None)` so
     the resolver can try the next source.
   - `Ok(Some(Probe::new(kind, value)))` for the happy path.

3. **Platform-gate the module** in `src/sources/mod.rs`:

   ```rust
   #[cfg(target_os = "yourplatform")]
   mod yourplatform;
   #[cfg(target_os = "yourplatform")]
   pub use yourplatform::YourSource;

   #[cfg(not(target_os = "yourplatform"))]
   mod yourplatform_stubs;
   #[cfg(not(target_os = "yourplatform"))]
   pub use yourplatform_stubs::YourSource;
   ```

   The stub module exists so callers can *name* the type on any
   platform; the stub's `probe()` returns `Ok(None)`. Follow
   `src/sources/macos_stubs.rs` — the `stub_source!` / `stub_impl!`
   macros from `stub_macros.rs` cover the boilerplate.

4. **Feature-gate optional sources** instead of platform-gating when
   the source is opt-in across all platforms (k8s, container). Add
   the feature to `Cargo.toml` under `[features]`, with a one-line
   comment describing what it enables and any dependencies it
   pulls in.

5. **Register the source.** See the [file-by-file checklist](#file-by-file-checklist)
   for everything that needs touching. The critical three:
   `SourceKind` variant, `source_kind_ids!` row, and
   `local_source_from_id` arm. Missing any of these gives a
   confusing partial integration.

6. **Insert into `default_chain()`** if appropriate (see
   [Decide whether it joins a default chain](#decide-whether-it-joins-a-default-chain)).

## Recipe B — cloud metadata (plaintext `CloudEndpoint`)

Use for providers that answer a single `GET` with the identifier as
the plaintext body. `src/sources/hetzner.rs` is the reference
implementation — ~30 lines, no generics work required.

1. **Create the module** `src/sources/<provider>.rs`:

   ```rust
   use crate::source::SourceKind;
   use crate::sources::cloud::{CloudEndpoint, CloudMetadata};

   pub type YourProviderMetadata<T> = CloudMetadata<YourProviderEndpoint, T>;

   pub struct YourProviderEndpoint;

   impl CloudEndpoint for YourProviderEndpoint {
       const DEBUG_NAME: &'static str = "YourProviderMetadata";
       const DEFAULT_BASE_URL: &'static str = "http://169.254.169.254";
       const PATH: &'static str = "/path/to/id";
       const KIND: SourceKind = SourceKind::YourProviderMetadata;

       fn headers() -> &'static [(&'static str, &'static str)] {
           &[]  // or &[("Metadata-Flavor", "Google")] etc.
       }
   }
   ```

2. **Add the feature** in `crates/host-identity/Cargo.toml`:

   ```toml
   yourprovider = ["_transport"]
   ```

   Always inherit `_transport` — never `dep:http` directly. Feature
   names are compact (no hyphens, Cargo limitation); the identifier
   string *is* hyphenated (see [Naming convention](../crates/host-identity/README.md#naming-convention)).

3. **Gate the module and re-export** in `src/sources/mod.rs`
   following the `hetzner` pattern (feature-gated `mod` + `pub use`).

4. **Wire into the network default chain.** Add one `chain.push(...)`
   to `network_default_chain` in `src/sources/mod.rs`, positioned
   per the
   [ordering principle](algorithm.md#ordering-principle). Keep the
   relative order of cloud providers stable (declaration order in
   `with_network_defaults` rustdoc).

5. **Register identifiers and `SourceKind`** per the
   [file-by-file checklist](#file-by-file-checklist). The cloud arm
   goes in `source_from_id_with_transport`, not
   `local_source_from_id`.

### Security requirements

Every cloud source inherits the transport contract documented in
`src/sources/cloud.rs`:

- **No redirects.** A 3xx from a compromised or spoofed metadata
  endpoint could forward provider-fingerprint headers
  (e.g. Azure's `Metadata: true`) off-cloud.
- **Short timeout** (single-digit seconds). Off-cloud hosts never
  answer — the timeout bounds resolver latency.

These are transport-level guarantees the *consumer* must provide, but
the source's module-level `//!` doc must restate them so consumers
reading the source rustdoc see the requirement.

## Recipe C — cloud metadata (bespoke, like AWS IMDSv2)

Only use this when recipe B genuinely can't express the protocol.
Examples: multi-step token exchanges, JSON decoding, or multiple
endpoints. Follow `src/sources/aws.rs` as the reference.

Same checklist as recipe B plus:

- Own the `Source` impl directly — don't use `CloudMetadata<E, T>`.
- Document every outcome (`Ok(None)` vs `Err(Error::Platform(...))`)
  in the source's `//!` doc and update
  [`docs/algorithm.md`](algorithm.md) with the probe-behaviour block.
- Distinguish "endpoint unreachable" (→ `Ok(None)`, fall through)
  from "endpoint responded but the response violates the documented
  contract" (→ `Err(Error::Platform(...))`, the caller should see it).

## Testing

Every source ships with unit tests in its own module. Cover at least:

- **Happy path** — a well-formed input yields `Ok(Some(probe))` with
  the expected `SourceKind` and raw value.
- **Missing input** — file absent, command missing, endpoint
  unreachable, wrong OS → `Ok(None)`.
- **Empty input** — empty or whitespace-only → `Ok(None)`.
- **Sentinel rejection** — the `uninitialized` string (any casing,
  trailing newline) → `Err(Error::Uninitialized { .. })`. Required
  for any source reading a machine-id-shaped file.
- **Malformed input** — whatever "malformed" means for the source.
  For cloud sources, a non-2xx status → `Ok(None)`.

### Cloud-source test transport

Use `sources::cloud::test_support::StubTransport` (see
`src/sources/hetzner.rs` tests for the shape). `StubTransport::new`
returns scripted responses; `StubTransport::shared` hands back the
captured requests so you can assert the method, URL, and headers the
source sent. Every cloud source must have a `hits_expected_path`
test that pins the exact URL and required headers — regressions in
header shape are silent until a provider rejects the request in
production.

### Integration tests

Add an integration test in `crates/host-identity/tests/integration.rs`
only when the new source changes resolver-level behaviour (joining
a default chain, interacting with other sources). Single-source
behaviour belongs in the source's own `#[cfg(test)] mod tests`.

### Testing discipline

Follow [the testing section](developer-guide.md#testing) of the
developer guide. In particular:

- No incidental coupling — don't hardcode paths or filenames the
  source doesn't branch on.
- Assert the specific value, not just `is_ok()`.
- Use `tempfile::NamedTempFile` / `TempDir` for filesystem fixtures.
- Gate env-var-touching tests behind `#[serial]`.

## Documentation updates

Each of these MUST be updated when adding a source — they're part of
the public contract and the CI docs build. Skipping any produces a
half-documented source that operators can configure but can't find
in the reference.

- **`src/sources/<name>.rs`** — module-level `//!` with the
  authoritative reference link and protocol description. Rustdoc on
  every public item (the crate sets `warn(missing_docs)`).
- **`src/source.rs`** — rustdoc on the new `SourceKind` variant,
  matching the format of existing variants.
- **`src/ids.rs`** — rustdoc on the new `source_ids` constant with
  a back-reference to the source type.
- **`crates/host-identity/README.md`** — feature row; cloud-provider
  table row for cloud sources.
- **`crates/host-identity/CHANGELOG.md`** — one line under
  `### Added` in the `## [Unreleased]` section. Name the source by
  its identifier string (`"hetzner-metadata"`) and feature flag.
- **`docs/algorithm.md`** — three updates:
  1. `default_chain` or `network_default_chain` listing.
  2. Per-source probe behaviour block under "Probe behaviours".
     Cloud sources sharing `CloudMetadata` can fold into the shared
     block; bespoke sources get their own.
  3. "External references" table entry with the authoritative link.
- **`README.md`** (repo root) — only if the top-level feature list
  names the provider. Skip for additions that slot into an existing
  enumeration (e.g. "AWS, GCP, Azure, DigitalOcean, Hetzner, OCI" —
  add your provider to the enumeration).

Do **not** hardcode counts anywhere ("7 cloud sources", "25 tests").
See [No stale counts](developer-guide.md#no-stale-counts).

## Pre-PR checks

From the workspace root:

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --workspace
```

Also run with your new feature alone, to catch
`#[cfg(feature = "_transport")]` gaps that `--all-features` hides:

```bash
cargo check --no-default-features
cargo check --no-default-features --features yourfeature
cargo test --no-default-features --features yourfeature
```

If the source is platform-native, cross-check on every relevant OS
you have access to. `#[cfg]`-gated stubs compile on other platforms
but only the native target actually runs `probe()` — an `unused
imports` warning or compile error that only fires on one OS is easy
to miss otherwise.

If you touched the CLI (`available_source_ids`, feature flags, or
anything surfaced by `host-identity sources`), run:

```bash
cargo run -p host-identity-cli -- sources
cargo run -p host-identity-cli -- resolve --sources <your-id>
```

and confirm the new identifier appears and resolves.

## Opening the PR

Follow [the workflow section](developer-guide.md#workflow):

1. Branch from `main`, commit atomically, run the clean-run commands
   above.
2. Conventional commit format. Scope is typically `sources`:

   ```text
   feat(sources): add <Provider> <kind> identity source
   ```

3. PR body includes:
   - What the source reads and the authoritative reference link.
   - Which feature gate it ships under.
   - Whether it joins a default chain and at what position.
   - Test coverage summary (the five cases from
     [Testing](#testing)).
   - `Fixes #NNN` if there's an issue.
4. The changelog entry under `## [Unreleased]` is part of the PR, not
   a follow-up. Reviewers check for it.
5. Reviews focus on correctness, API shape, sentinel handling, and
   test coverage before style. Expect questions about chain position
   and identifier naming — those are the hardest things to change
   after release.

## Appendix — files that list every source

When the list in [File-by-file checklist](#file-by-file-checklist)
has drifted, these greps find every enumeration site. Run them with
the name of an existing source (e.g. `Hetzner`, `hetzner`,
`HETZNER_METADATA`) and mirror every hit for your new source.

```bash
rg -n 'Hetzner|hetzner|HETZNER_METADATA' \
    crates/ docs/ README.md
```

Expected hits (as of this writing):

- `crates/host-identity/src/source.rs` — `SourceKind` variant and
  `source_kind_ids!` row.
- `crates/host-identity/src/ids.rs` — `source_ids` constant,
  `source_from_id_with_transport` arm, round-trip test.
- `crates/host-identity/src/sources/mod.rs` — `mod` + `pub use` +
  `network_default_chain` entry.
- `crates/host-identity/src/sources/<provider>.rs` — the module.
- `crates/host-identity/src/lib.rs` — cloud-metadata table.
- `crates/host-identity/Cargo.toml` — feature flag.
- `crates/host-identity/README.md` — feature row, cloud table row.
- `crates/host-identity/CHANGELOG.md` — `Added` entry.
- `crates/host-identity-cli/src/lib.rs` — `available_source_ids`
  entry.
- `crates/host-identity-cli/Cargo.toml` — `network` feature.
- `docs/algorithm.md` — default chain listing, probe-behaviour
  block, references table.
- `README.md` — top-level provider enumeration.
- `.github/workflows/ci.yml` — any per-feature matrix entry that
  mentions sibling providers by name.

If your source is platform-native (not cloud), grep for an
equivalent exemplar (`MachineId`, `FreeBsdHostId`, `IllumosHostId`)
instead — the set of files is similar but without the `_transport`
and CI-matrix rows.
