# host-identity

A Rust library that returns a stable, distinct UUID for the host
your program is running on — and doesn't quietly collide on cloned
VMs, LXC guests, Docker containers, Red Hat images, systemd's
`uninitialized` sentinel, or minimal images missing `/etc/machine-id`.

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
host-identity = "0.1"
```

Opt-in cloud metadata (AWS IMDSv2, GCP, Azure, DigitalOcean, Hetzner,
OCI) and Kubernetes sources live behind feature flags. Network sources
are generic over an `HttpTransport` trait; the crate ships no HTTP
client, so it slots into whatever stack you already run on.

A `hostid` CLI is published as a separate binary crate:

```bash
cargo install host-identity-cli
hostid                       # print the resolved UUID
hostid audit                 # show what every source produced
```

## Why not `machine-uid` or a one-liner?

Most host-ID crates — and most agents — do one thing: read
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
platform-appropriate source chain with explicit container awareness,
sentinel and empty-file rejection, an `HOST_IDENTITY` env / file
override, deterministic UUID v5 wrapping, and no random-UUID
fallback on total failure (callers decide). If you just need
`/etc/machine-id`, `machine-uid` is smaller; if you need an ID you
can ship an observability pipeline on, read on.

See [`crates/host-identity/README.md`](crates/host-identity/README.md)
for the problem statement in full and the API surface, and
[`docs/algorithm.md`](docs/algorithm.md) for the resolution algorithm.

## Workspace layout

| Crate                                                         | Purpose                                                   |
| ------------------------------------------------------------- | --------------------------------------------------------- |
| [`host-identity`](crates/host-identity)                       | Library: composable source chain that resolves to a UUID. |
| [`host-identity-cli`](crates/host-identity-cli) (`hostid`)    | Command-line wrapper over the library.                    |

## Packaging

Build artifacts the `hostid` CLI ships:

- Binary: `target/release/hostid` (after `cargo build --release -p host-identity-cli`)
- Man page: `man/hostid.1` plus one page per subcommand
  (`man/hostid-resolve.1`, `man/hostid-audit.1`, `man/hostid-sources.1`),
  committed in-repo and regenerated with `cargo xtask`.

Standard install locations:

    install -Dm755 target/release/hostid "$PREFIX/bin/hostid"
    install -Dm644 man/hostid.1          "$PREFIX/share/man/man1/hostid.1"
    install -Dm644 man/hostid-resolve.1  "$PREFIX/share/man/man1/hostid-resolve.1"
    install -Dm644 man/hostid-audit.1    "$PREFIX/share/man/man1/hostid-audit.1"
    install -Dm644 man/hostid-sources.1  "$PREFIX/share/man/man1/hostid-sources.1"

The man pages are generated from the `clap` command definition via
`clap_mangen`. Packagers do not need to run the generator; the committed
`.1` files are the source of truth for distribution. A CI job fails if
the committed pages drift from the current `clap` metadata.

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
