# host-identity

[![CI](https://github.com/dekobon/host-identity/actions/workflows/ci.yml/badge.svg?branch=main&event=push)](https://github.com/dekobon/host-identity/actions/workflows/ci.yml?query=branch%3Amain+event%3Apush)
[![CodeQL](https://github.com/dekobon/host-identity/actions/workflows/github-code-scanning/codeql/badge.svg)](https://github.com/dekobon/host-identity/actions/workflows/github-code-scanning/codeql)

A Rust library that returns a stable, distinct UUID for the host
your program is running on — and doesn't quietly collide on cloned
VMs, LXC guests, Docker containers, Red Hat images, systemd's
`uninitialized` sentinel, or minimal images missing `/etc/machine-id`.

An **identity source** is any single mechanism the crate can probe
to learn who the host is — `/etc/machine-id`, the SMBIOS product
UUID, the Windows registry's `MachineGuid`, AWS IMDSv2, a Kubernetes
pod UID, an `HOST_IDENTITY` env override, a caller-supplied closure.
The library composes identity sources into an ordered chain; the
first one that produces a usable value wins.

```rust
use host_identity::resolve;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let id = resolve()?;  // UUID v5 under a stable namespace
    println!("{id}");
    Ok(())
}
```

```toml
[dependencies]
host-identity = "1.0"
```

Opt-in cloud metadata (AWS IMDSv2, GCP, Azure, DigitalOcean, Hetzner,
OCI) and Kubernetes identity sources live behind feature flags.
Network-backed identity sources are generic over an `HttpTransport`
trait; the crate ships no HTTP client, so it slots into whatever stack
you already run on.

A `hostid` CLI is published as a separate binary crate:

```bash
cargo install host-identity-cli
hostid                       # print the resolved UUID
hostid audit                 # show what every identity source produced
```

## Why not `machine-uid` or a one-liner?

Most host-ID crates (and most other libraries) do one thing: read
`/etc/machine-id` (or the OS equivalent) and hand back whatever bytes
are there. That's fine when the file exists, is populated, and isn't
shared across hosts. In real fleets it fails in half a dozen ways
that look like nothing is wrong until weeks of telemetry are already
corrupted:

- Cloned VM templates ship with the same `machine-id`.
- LXC guests inherit the host's ID; Red Hat containers bind-mount it.
- Docker doesn't mount `machine-id` at all; naive fallbacks mint a
  fresh random UUID on every restart.
- systemd writes the literal string `uninitialized` during first
  boot — a hash of that is still a collision.
- Minimal images (distroless, some Alpine configs) don't have the
  file.
- When things do go wrong, there's usually no operator override.

`host-identity` treats host identity as a layered problem: a
platform-appropriate chain of identity sources with explicit container
awareness, sentinel and empty-file rejection, an `HOST_IDENTITY` env /
file override, deterministic UUID v5 wrapping, and no random-UUID
fallback on total failure (callers decide). If you just need
`/etc/machine-id`, `machine-uid` is smaller; if you need an ID you
can ship an observability pipeline on, read on.

### Identity scope: host is not container

Most naive machine-ID libraries conflate "the host" with "the thing
running on the host". On a plain server those coincide; inside a
container on a cloud VM inside Kubernetes they do not. `host-identity`
models identity at four distinct scopes and lets the chain pick the
most specific one available:

- **per-pod** — `KubernetesPodUid` (distinct per pod).
- **per-container** — `ContainerId`, `LxcId` (distinct per container
  runtime instance; `LxcId` covers LXC/LXD name-based deployments).
- **per-instance** — cloud metadata (`AwsImds`, `GcpMetadata`, …),
  SMBIOS product UUID, macOS `IOPlatformUUID` (distinct per VM;
  shared by every container on that VM).
- **per-host-OS** — `/etc/machine-id`, Windows `MachineGuid`, BSD
  `hostid` (distinct per OS install; shared by every container that
  inherits or bind-mounts the file).

The built-in chains order sources per-pod → per-container →
per-instance → per-host-OS, so a process running inside a container
on an EC2 instance resolves to its container ID rather than
collapsing onto the instance ID it shares with every sibling
container. See [`docs/algorithm.md` → "Identity scope"](docs/algorithm.md#identity-scope-what-host-means-per-source)
for the full rules and the traps to avoid when building a custom
chain.

See [`crates/host-identity/README.md`](crates/host-identity/README.md)
for the problem statement in full and the API surface, and
[`docs/algorithm.md`](docs/algorithm.md) for the resolution algorithm.

## Workspace layout

| Crate                                                         | Purpose                                                            |
| ------------------------------------------------------------- | ------------------------------------------------------------------ |
| [`host-identity`](crates/host-identity)                       | Library: composable identity-source chain that resolves to a UUID. |
| [`host-identity-cli`](crates/host-identity-cli) (`hostid`)    | Command-line wrapper over the library.                             |

## Install

Every release tag (`v*`) publishes prebuilt binaries and native
packages for the common desktop/server targets to
[GitHub Releases](https://github.com/dekobon/host-identity/releases).

| Platform               | Install                                                                      |
| ---------------------- | ---------------------------------------------------------------------------- |
| Debian / Ubuntu        | `apt install ./hostid_<ver>_amd64.deb` (or `arm64`)                          |
| RHEL / Rocky / Fedora / Amazon Linux | `dnf install ./hostid-<ver>-1.x86_64.rpm` (or `aarch64`)       |
| Alpine                 | `apk add --allow-untrusted ./hostid-<ver>-r0.apk`                            |
| FreeBSD (amd64)        | `pkg install ./hostid-<ver>.pkg`                                             |
| macOS (Intel + Apple Silicon) | `brew install dekobon/host-identity/hostid`                           |
| Windows (x86_64 / arm64) | `scoop install hostid` (after `scoop bucket add dekobon https://github.com/dekobon/scoop-bucket`) or `winget install dekobon.hostid` |
| Portable (Linux / macOS / Windows) | Download `hostid-<ver>-<target>.tar.gz` / `.zip` and extract    |
| From source            | `cargo install host-identity-cli`                                            |

Every package installs the binary at `hostid` and the man pages
(`hostid(1)`, `hostid-resolve(1)`, `hostid-audit(1)`,
`hostid-sources(1)`) under the distro's standard `man1/` directory.

Release artefacts are signed. Verify with `minisign` against the
committed [`minisign.pub`](minisign.pub) and with GitHub's SLSA
attestations:

```bash
minisign -Vm SHA256SUMS -p minisign.pub
gh attestation verify hostid-<ver>-<target>.tar.gz -R dekobon/host-identity
```

Gaps on v1: FreeBSD `aarch64` is not prebuilt (tier-3 Rust target) —
use `cargo install` or the FreeBSD ports tree. macOS direct-download
tarballs are unsigned/unnotarized; if Gatekeeper quarantines them,
run `xattr -d com.apple.quarantine ./hostid` once. Homebrew installs
are unaffected.

## Packaging

Building a package? See [`docs/packaging.md`](docs/packaging.md) for
build artefacts, the standard install layout, the release-pipeline
stages, per-distro notes, and signing / parity checks.

## License

Dual-licensed under [Apache License 2.0](LICENSE-APACHE) or
[MIT](LICENSE-MIT), at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in this project by you, as defined in the
Apache-2.0 license, shall be dual-licensed as above, without any
additional terms or conditions.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the onboarding summary
and [`docs/developer-guide.md`](docs/developer-guide.md) for the full
conventions.
