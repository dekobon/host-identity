# Packaging

Reference for anyone producing a `host-identity` build artefact — whether you
are packaging for a distro, rolling a private internal build, or
debugging why a release-workflow stage failed. End users looking for
install instructions want [`install.md`](install.md) instead.

## Contents

- [Build artefacts](#build-artefacts)
- [Standard install layout](#standard-install-layout)
- [Man pages](#man-pages)
- [Release pipeline overview](#release-pipeline-overview)
- [Distro-specific notes](#distro-specific-notes)
- [Signing and provenance](#signing-and-provenance)
- [Version and parity checks](#version-and-parity-checks)

## Build artefacts

A full `host-identity` build ships three kinds of files:

| Artefact        | Source                                                         |
| --------------- | -------------------------------------------------------------- |
| Binary          | `cargo build --release -p host-identity-cli` → `target/<target>/release/host-identity` |
| Man pages       | `man/host-identity.1`, `man/host-identity-resolve.1`, `man/host-identity-audit.1`, `man/host-identity-sources.1` (committed; regenerate with `cargo xtask`) |
| Debug symbols   | `target/<target>/release/host-identity.dbg` (Unix, via `objcopy --only-keep-debug`) or `host-identity.pdb` (Windows) |
| Third-party licenses | `THIRD-PARTY-LICENSES.md` — generated from [`about.toml`](../about.toml) and [`about.hbs`](../about.hbs) by `cargo about generate` per target |

The committed `.1` files are the source of truth for distribution;
packagers should not need a Rust toolchain just to regenerate them.
CI fails the `preflight` job if the committed pages drift from the
current `clap` metadata, so a PR that changes CLI flags must include
the regenerated pages.

## Standard install layout

```bash
install -Dm755 target/release/host-identity "$PREFIX/bin/host-identity"
install -Dm644 man/host-identity.1          "$PREFIX/share/man/man1/host-identity.1"
install -Dm644 man/host-identity-resolve.1  "$PREFIX/share/man/man1/host-identity-resolve.1"
install -Dm644 man/host-identity-audit.1    "$PREFIX/share/man/man1/host-identity-audit.1"
install -Dm644 man/host-identity-sources.1  "$PREFIX/share/man/man1/host-identity-sources.1"
install -Dm644 LICENSE-APACHE        "$PREFIX/share/doc/host-identity/LICENSE-APACHE"
install -Dm644 LICENSE-MIT           "$PREFIX/share/doc/host-identity/LICENSE-MIT"
install -Dm644 THIRD-PARTY-LICENSES.md "$PREFIX/share/doc/host-identity/THIRD-PARTY-LICENSES.md"
```

`THIRD-PARTY-LICENSES.md` carries the attributions for every crate
linked into the `host-identity` binary. It is generated from
[`about.toml`](../about.toml) (license allowlist + target list) and
[`about.hbs`](../about.hbs) (Markdown template) by `cargo-about`
during the release workflow's `build` stage. Packagers producing an
out-of-band build should regenerate it first:

```bash
cargo install cargo-about
cargo about generate --locked \
    --manifest-path crates/host-identity-cli/Cargo.toml \
    about.hbs > THIRD-PARTY-LICENSES.md
```

FreeBSD packages install under `/usr/local/` rather than `/usr/`; see
[`packaging/freebsd/port/`](../packaging/freebsd) for the canonical
prefix and license-file placement (`share/licenses/host-identity/`).

## Man pages

Man pages are generated from the `clap` command definition via
`clap_mangen`, driven by the workspace task runner in
[`xtask/src/main.rs`](../xtask/src/main.rs):

```bash
cargo xtask            # rewrites man/*.1
```

Regenerate whenever a CLI flag, subcommand, or help string changes.
The files are committed so downstream packagers do not pull in
`clap_mangen` as a build-time dependency.

## Release pipeline overview

Release automation lives in [`.github/workflows/release.yml`](../.github/workflows/release.yml)
and is triggered by pushing a `v*` tag. `workflow_dispatch` re-runs
an existing tag for rehearsal; the pre-release gate prevents tap /
bucket updates from leaking during rehearsal.

Stages, in order:

1. **preflight** — resolve the tag, verify `Cargo.toml` ↔ tag parity,
   confirm `minisign.pub` is not the placeholder, check man-page
   freshness, extract the release notes from `CHANGELOG.md`, and
   classify the release (stable vs pre-release).
2. **build** — cross-compile per target, stage archive contents,
   strip the binary and split debug symbols, produce the per-target
   `.tar.gz` / `.zip`.
3. **package-deb / package-rpm / package-apk / package-freebsd** —
   consume the staged archive and emit native packages by rendering
   the templates under [`packaging/`](../packaging).
4. **Tap / bucket updates** — Homebrew tap and Scoop bucket pushes.
5. **Attestations and signing** — SLSA attestations via `gh
   attestation`, plus `minisign` signatures over `SHA256SUMS`.

The template directory [`packaging/`](../packaging) documents every
`@@TOKEN@@` placeholder and which release stage substitutes it.

## Distro-specific notes

### Debian / Ubuntu (`cargo-deb`)

Driven by `package-deb`. The stage unpacks the archive into the layout
`cargo-deb` expects, so the `Cargo.toml` `[package.metadata.deb]` entries
are the source of truth for control-file contents. Ship both `amd64`
and `arm64`.

### RHEL / Fedora / Rocky / Amazon Linux (`cargo-generate-rpm`)

Driven by `package-rpm`. Analogous to the Debian flow but keyed off
`[package.metadata.generate-rpm]`. Ship `x86_64` and `aarch64`.

### Alpine (`abuild` in `alpine:3.20`)

Driven by `package-apk`. Renders
[`packaging/alpine/APKBUILD.in`](../packaging/alpine) with
`@@VERSION@@`, `@@ARCH@@`, `@@SHA512@@`, then runs `abuild -r` inside
the container. The apk is unsigned; end users install with
`--allow-untrusted` or verify via the detached minisign signature.

### FreeBSD (`pkg create`)

Driven by `package-freebsd` in a FreeBSD VM job. Renders
[`packaging/freebsd/+MANIFEST.in`](../packaging/freebsd) and the ports
`Makefile`, stages under `/usr/local/`, and runs `pkg create -M`. Only
`amd64` is prebuilt; `aarch64` users build via `cargo install` or the
ports tree.

### Homebrew

[`packaging/homebrew/host-identity.rb.tmpl`](../packaging/homebrew) is pushed
to the external `homebrew-host-identity` tap with the per-target
tarball SHA-256s substituted in. The formula installs both `amd64`
and `arm64` bottles.

### Scoop

[`packaging/scoop/host-identity.json.in`](../packaging/scoop) is pushed to the
external `scoop-bucket` repo. Covers `x86_64` and `arm64` on Windows.

## Signing and provenance

Every release tarball is covered by:

- A detached minisign signature over `SHA256SUMS`, verifiable with the
  committed [`minisign.pub`](../minisign.pub).
- A GitHub SLSA provenance attestation, verifiable with
  `gh attestation verify <artefact> -R dekobon/host-identity`.

Windows binaries are not Authenticode-signed; macOS direct-download
tarballs are neither signed nor notarized. Homebrew installs run
through their respective trust store and are unaffected by
Gatekeeper quarantine.

## Version and parity checks

Pre-release checks the packaging pipeline enforces:

- `Cargo.toml` `version` must equal the tag (minus the leading `v`).
- `man/*.1` must match the current `clap` metadata (re-run
  `cargo xtask` if this fails).
- `minisign.pub` must not be the placeholder checked in at project
  bootstrap.
- `CHANGELOG.md` must contain a section for the tag; its contents
  become the GitHub Release body.

If any of these fail, fix them on the release branch and re-tag
rather than patching the workflow — the checks exist because each
failure mode has bitten a previous release.
