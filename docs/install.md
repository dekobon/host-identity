# Installing host-identity

End-user install guide for the `host-identity` CLI. For embedding the
library, add `host-identity` to your `Cargo.toml` — see the top-level
[`README.md`](../README.md). For building your own package artefacts,
see [`packaging.md`](packaging.md).

Every release tag (`v*`) publishes prebuilt binaries, native packages,
and a Homebrew/Scoop manifest to
[GitHub Releases](https://github.com/dekobon/host-identity/releases).

## Contents

- [Quick reference](#quick-reference)
- [From source (`cargo install`)](#from-source-cargo-install)
- [Debian and Ubuntu](#debian-and-ubuntu)
- [RHEL, Rocky, Fedora, Amazon Linux](#rhel-rocky-fedora-amazon-linux)
- [Alpine](#alpine)
- [FreeBSD](#freebsd)
- [macOS (Homebrew)](#macos-homebrew)
- [Windows (Scoop)](#windows-scoop)
- [Portable tarball / zip](#portable-tarball--zip)
- [Verifying a release](#verifying-a-release)
- [Upgrading and uninstalling](#upgrading-and-uninstalling)
- [Platform coverage and known gaps](#platform-coverage-and-known-gaps)

## Quick reference

| Platform                           | Install                                                                                                   |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------- |
| Any (Rust toolchain)               | `cargo install --locked host-identity-cli`                                                                |
| Debian / Ubuntu                    | `apt install ./host-identity_<ver>_amd64.deb` (or `arm64`)                                                |
| RHEL / Rocky / Fedora / Amazon Linux | `dnf install ./host-identity-<ver>-1.x86_64.rpm` (or `aarch64`)                                         |
| Alpine                             | `apk add --allow-untrusted ./host-identity-<ver>-r0.apk`                                                  |
| FreeBSD (amd64)                    | `pkg install ./host-identity-<ver>.pkg`                                                                   |
| macOS (Apple Silicon)              | `brew install dekobon/host-identity/host-identity`                                                        |
| macOS (Intel)                      | `cargo install --locked host-identity-cli` (or the Apple-silicon build under Rosetta 2)                   |
| Windows (x86_64 / arm64)           | `scoop bucket add dekobon https://github.com/dekobon/scoop-bucket && scoop install host-identity`         |
| Portable (Linux / macOS / Windows) | Download `host-identity-<ver>-<target>.tar.gz` / `.zip` and extract                                       |

Every package installs the binary at `host-identity` and the man pages
(`host-identity(1)`, `host-identity-resolve(1)`,
`host-identity-audit(1)`, `host-identity-sources(1)`) under the
distro's standard `man1/` directory.

## From source (`cargo install`)

The most portable path — works anywhere Rust does, including platforms
without a prebuilt binary (Intel macOS, FreeBSD `aarch64`, Linux
distros outside the deb/rpm/apk families).

```bash
cargo install --locked host-identity-cli
```

Notes:

- The crate name (`host-identity-cli`) differs from the binary name
  (`host-identity`) and from the library crate (`host-identity`).
  `cargo install host-identity` has no binary target and will fail —
  always use the `-cli` suffix.
- `--locked` makes cargo honour the committed `Cargo.lock`. Recommended
  for every release install so you get the exact dependency graph CI
  tested.
- Default features include both local sources (machine-id, DMI,
  container) and network sources (cloud metadata, Kubernetes). For a
  strictly local build:

  ```bash
  cargo install --locked host-identity-cli \
      --no-default-features --features container
  ```

- A `cargo install` does **not** install man pages. Download them from
  the matching release tarball (`man/*.1`) if you want them on the
  `MANPATH`.

## Debian and Ubuntu

Download the `.deb` matching your architecture from the release page,
then:

```bash
sudo apt install ./host-identity_<ver>_amd64.deb      # or _arm64.deb
```

`apt install ./…` handles dependency resolution; `dpkg -i` works too
but will not pull in missing dependencies. Architectures shipped:
`amd64`, `arm64`.

## RHEL, Rocky, Fedora, Amazon Linux

```bash
sudo dnf install ./host-identity-<ver>-1.x86_64.rpm   # or .aarch64.rpm
```

On systems with `yum` rather than `dnf`, substitute `yum install`.
Architectures shipped: `x86_64`, `aarch64`.

## Alpine

The apk is unsigned (no abuild key is published), so install with
`--allow-untrusted` and verify integrity via the detached `minisign`
signature — see [Verifying a release](#verifying-a-release).

```bash
sudo apk add --allow-untrusted ./host-identity-<ver>-r0.apk
```

Architectures shipped: `x86_64`, `aarch64`.

## FreeBSD

```bash
sudo pkg install ./host-identity-<ver>.pkg
```

Only `amd64` is prebuilt. FreeBSD `aarch64` is a tier-3 Rust target;
install via `cargo install --locked host-identity-cli` or the FreeBSD
ports tree (see [`packaging/freebsd/port/`](../packaging/freebsd/port)).

## macOS (Homebrew)

The Homebrew formula lives in an external tap repo:
<https://github.com/dekobon/homebrew-host-identity>

Homebrew refers to taps with a `<user>/<repo>` shorthand and auto-prepends
`homebrew-` to the repo segment when resolving the GitHub URL. So:

| Shorthand                            | Resolves to                                           |
| ------------------------------------ | ----------------------------------------------------- |
| `dekobon/host-identity`              | `github.com/dekobon/homebrew-host-identity`           |
| `dekobon/host-identity/host-identity` | the `host-identity` formula inside that tap           |

### One-shot install (recommended)

The three-slash form auto-taps the repo and installs the formula in one
step:

```bash
brew install dekobon/host-identity/host-identity
```

### Tap first, then install

Equivalent, just explicit. The second argument to `brew tap` pins the
URL — useful if you want the exact repo written into your Homebrew
state, or if you're installing from a fork or mirror:

```bash
brew tap dekobon/host-identity https://github.com/dekobon/homebrew-host-identity
brew install host-identity
```

Without the explicit URL (`brew tap dekobon/host-identity`), Homebrew
infers the same GitHub URL via the `homebrew-` convention above.

### Upgrade and uninstall

```bash
brew upgrade host-identity
brew uninstall host-identity
brew untap dekobon/host-identity   # optional — drops the tap itself
```

### Apple Silicon only

The formula ships **only** the `aarch64-apple-darwin` bottle. Intel
Macs (`x86_64-apple-darwin`) are not covered by Homebrew. The
background: GitHub's `macos-13` (Intel) runner pool is chronically
oversubscribed, and the queue depth made rehearsal tags unreliable.
Apple stopped selling Intel Macs in late 2023, so the remaining Intel
user base has two good workarounds:

1. **`cargo install`** — the most robust path; builds a native
   `x86_64` binary.

   ```bash
   cargo install --locked host-identity-cli
   ```

2. **Rosetta 2** — the Apple-silicon build runs under Rosetta on Intel
   Macs without modification. Download
   `host-identity-<ver>-aarch64-apple-darwin.tar.gz` from the release
   page and extract. Rosetta handles the translation transparently.

### Direct tarball and Gatekeeper

macOS direct-download tarballs are neither Authenticode-signed nor
notarized. If Gatekeeper quarantines the binary after extraction:

```bash
xattr -d com.apple.quarantine ./host-identity
```

Homebrew installs go through Homebrew's trust store and are unaffected
by this.

### Linuxbrew

Not supported. The formula uses `on_macos` / `on_arm` gates and ships
no Linux bottle. Use the `.deb`, `.rpm`, `.apk`, or `cargo install`
instead.

## Windows (Scoop)

The manifest lives in the external bucket
[`dekobon/scoop-bucket`](https://github.com/dekobon/scoop-bucket). Add
the bucket once, then install:

```powershell
scoop bucket add dekobon https://github.com/dekobon/scoop-bucket
scoop install host-identity
```

Covers both `x86_64` and `aarch64` on Windows. Upgrade with
`scoop update host-identity`; uninstall with `scoop uninstall
host-identity`.

Windows binaries are not Authenticode-signed. SmartScreen may warn on
first launch of the extracted portable `.zip`; the Scoop path runs
through Scoop's own hash verification.

## Portable tarball / zip

Every release ships `host-identity-<ver>-<target>.tar.gz` (Unix) or
`.zip` (Windows) for every supported target triple. The archive
contains:

```text
host-identity(.exe)       # binary
man/*.1                   # man pages (Unix targets)
README.md
LICENSE-APACHE
LICENSE-MIT
THIRD-PARTY-LICENSES.md
```

Extract and drop the binary somewhere on your `PATH`. The standard
Unix layout is:

```bash
install -Dm755 host-identity          /usr/local/bin/host-identity
install -Dm644 man/host-identity.1    /usr/local/share/man/man1/host-identity.1
# ... repeat for host-identity-resolve.1, -audit.1, -sources.1
```

Targets shipped: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`,
`aarch64-apple-darwin`, `x86_64-pc-windows-msvc`,
`aarch64-pc-windows-msvc`, `x86_64-unknown-freebsd`.

## Verifying a release

Release artefacts are covered by two independent signatures.

### minisign

Every release publishes `SHA256SUMS` and a detached
`SHA256SUMS.minisig`. Verify with the committed
[`minisign.pub`](../minisign.pub):

```bash
# Download SHA256SUMS, SHA256SUMS.minisig, and the artefact(s).
minisign -Vm SHA256SUMS -p minisign.pub
sha256sum -c SHA256SUMS --ignore-missing
```

### GitHub SLSA attestation

Every artefact also has a SLSA build provenance attestation attached
via GitHub's attestation store. Verify with the `gh` CLI:

```bash
gh attestation verify host-identity-<ver>-<target>.tar.gz \
    -R dekobon/host-identity
```

The attestation proves the artefact was produced by the
`release.yml` workflow in this repo, tied to the exact commit SHA of
the release tag.

## Upgrading and uninstalling

| Install method | Upgrade                                     | Uninstall                                  |
| -------------- | ------------------------------------------- | ------------------------------------------ |
| `cargo install`| `cargo install --locked host-identity-cli --force` | `cargo uninstall host-identity-cli`  |
| deb            | `sudo apt install ./host-identity_<ver>_<arch>.deb` | `sudo apt remove host-identity`    |
| rpm            | `sudo dnf upgrade ./host-identity-<ver>-1.<arch>.rpm` | `sudo dnf remove host-identity`  |
| apk            | `sudo apk add --allow-untrusted ./host-identity-<ver>-r0.apk` | `sudo apk del host-identity` |
| FreeBSD pkg    | `sudo pkg upgrade ./host-identity-<ver>.pkg` | `sudo pkg delete host-identity`            |
| Homebrew       | `brew upgrade host-identity`                | `brew uninstall host-identity`             |
| Scoop          | `scoop update host-identity`                | `scoop uninstall host-identity`            |
| Portable       | Replace the extracted binary                | Delete the extracted binary                |

## Platform coverage and known gaps

- **FreeBSD `aarch64`** — not prebuilt (tier-3 Rust target). Use
  `cargo install` or the ports tree.
- **macOS Intel (`x86_64-apple-darwin`)** — not prebuilt; no Homebrew
  bottle. Use `cargo install --locked host-identity-cli` or run the
  Apple-silicon binary under Rosetta 2.
- **Linuxbrew** — not supported; the Homebrew formula is macOS-only.
- **Windows Authenticode / macOS notarization** — not applied to
  direct-download archives. Package-manager installs (Homebrew, Scoop)
  go through their own trust stores and are unaffected.

If you hit a platform gap the docs do not cover, please
[open an issue](https://github.com/dekobon/host-identity/issues) — a
good bug report against an under-tested target is the single most
useful contribution you can make.
