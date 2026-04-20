# host-identity

[![CI](https://github.com/dekobon/host-identity/actions/workflows/ci.yml/badge.svg?branch=main&event=push)](https://github.com/dekobon/host-identity/actions/workflows/ci.yml?query=branch%3Amain+event%3Apush)
[![CodeQL](https://github.com/dekobon/host-identity/actions/workflows/github-code-scanning/codeql/badge.svg)](https://github.com/dekobon/host-identity/actions/workflows/github-code-scanning/codeql)
[![crates.io](https://img.shields.io/crates/v/host-identity.svg)](https://crates.io/crates/host-identity)

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
OCI, OpenStack) and Kubernetes identity sources live behind feature
flags.
Network-backed identity sources are generic over an `HttpTransport`
trait; the crate ships no HTTP client, so it slots into whatever stack
you already run on.

A `host-identity` CLI is published as a separate binary crate:

```bash
cargo install host-identity-cli
host-identity                       # print the resolved UUID
host-identity audit                 # show what every identity source produced
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
awareness, sentinel, empty-file, and DMI vendor-placeholder rejection,
an `HOST_IDENTITY` env /
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

### App-specific derivation (privacy)

Exporting a raw host identifier to telemetry, crash reports, or
third-party analytics is exactly the pattern systemd's
[`machine-id(5)`](https://www.freedesktop.org/software/systemd/man/machine-id.html)
spec warns against: every app that reads the raw value gets the same
cross-correlatable ID for the host. `AppSpecific<S>` wraps any
inner source and HMAC-derives a per-app UUID from its raw value:

```rust
use host_identity::sources::{AppSpecific, MachineIdFile};
use host_identity::Resolver;

let id = Resolver::new()
    .push(AppSpecific::new(
        MachineIdFile::default(),
        b"com.example.telemetry".to_vec(),
    ))
    .resolve()?;
# Ok::<(), host_identity::Error>(())
```

Two apps on the same host with different `app_id`s get uncorrelatable
IDs; the raw machine-id never leaves the process. See
[`docs/developer-guide.md` → "App-specific derivation"](docs/developer-guide.md#app-specific-derivation).

The CLI exposes the same derivation via `--app-id`, which wraps every
source in the chain:

```sh
host-identity resolve --app-id com.example.telemetry
host-identity resolve --app-id com.example.telemetry --format json
```

Source labels in the output become `app-specific:<inner>` (for example
`app-specific:machine-id`). Pair with `--wrap passthrough` to emit the
byte-exact AppSpecific UUID; the default `--wrap v5` additionally
re-hashes it under this crate's namespace.

## Workspace layout

| Crate                                                         | Purpose                                                            |
| ------------------------------------------------------------- | ------------------------------------------------------------------ |
| [`host-identity`](crates/host-identity)                       | Library: composable identity-source chain that resolves to a UUID. |
| [`host-identity-cli`](crates/host-identity-cli) (`host-identity`) | Command-line wrapper over the library.                         |

## Install

If you have a Rust toolchain:

```bash
cargo install --locked host-identity-cli
```

Every release tag (`v*`) also publishes prebuilt binaries and native
packages (`.deb`, `.rpm`, `.apk`, FreeBSD `.pkg`, Homebrew bottle,
Scoop manifest, portable `.tar.gz` / `.zip`) to
[GitHub Releases](https://github.com/dekobon/host-identity/releases).
See [`docs/install.md`](docs/install.md) for per-platform install
commands, signature verification, upgrade/uninstall recipes, and the
known platform-coverage gaps.

## Packaging

Building a package? See [`docs/packaging.md`](docs/packaging.md) for
build artefacts, the standard install layout, the release-pipeline
stages, per-distro notes, and signing / parity checks.

## A note on platform coverage

`host-identity` targets a wide matrix — Linux distros, macOS, Windows,
FreeBSD, several cloud metadata services, LXC/LXD, Kubernetes — but
the author maintains it on a small set of personal machines and has
limited resources to exercise every supported platform on real
hardware or paid cloud accounts. CI covers what it can; the long tail
inevitably slips through.

If you hit a bug on a platform or environment the test matrix misses,
please [open an issue](https://github.com/dekobon/host-identity/issues)
with the details — it is the single most useful contribution you can
make. Donations of testing environments (cloud credits, access to
uncommon hardware, BSD or Windows build hosts) are also very
welcome; reach out via an issue if you can help.

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
