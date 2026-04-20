# `host-identity` — a CLI for [`host-identity`]

Resolve a stable, collision-resistant host UUID across platforms, container
runtimes, cloud providers, and Kubernetes. This crate ships the `host-identity`
binary as a thin wrapper over the [`host-identity`] library; use the library
directly if you need to embed the same logic in another program.

## Install

```bash
cargo install host-identity-cli
```

This gives you the `host-identity` executable on your `PATH`. The binary
is called `host-identity` to avoid colliding with coreutils `hostid(1)`,
which ships on Linux and the BSDs. The crate name (`host-identity-cli`)
differs from the binary name (`host-identity`) and from the library
crate name (`host-identity`) — `cargo install host-identity` has no
binary and will not do what you want; always use the `-cli` suffix.

Default features enable both local (machine-id, DMI, container) and
network (cloud metadata, Kubernetes) sources. To build a strictly local
binary:

```bash
cargo install host-identity-cli --no-default-features --features container
```

## Usage

```bash
# Print the host UUID (default chain, local sources only).
host-identity

# Same, but include cloud-metadata and Kubernetes sources.
host-identity resolve --network

# Walk every source without short-circuiting — useful for diagnostics.
host-identity audit

# List every source identifier compiled into this binary.
host-identity sources

# Build a custom chain from source identifiers.
host-identity resolve --sources env-override,machine-id,dmi

# Machine-readable output.
host-identity resolve --format json
host-identity audit --format json
```

### Flags

| Flag                 | Values                          | Default | Notes                                                      |
| -------------------- | ------------------------------- | ------- | ---------------------------------------------------------- |
| `--format`           | `plain`, `summary`, `json`      | `plain` | `summary` prints `source:uuid`; `plain` prints only UUID.  |
| `--wrap`             | `v5`, `v3`, `passthrough`       | `v5`    | How the raw ID becomes a UUID — see [Wrap strategies](#wrap-strategies). |
| `--sources <ids>`    | comma-separated source IDs      | *(unset)* | Build a custom chain; see `host-identity sources`.              |
| `--network`          | *(flag)*                        | off     | Adds cloud / k8s sources (requires `network` feature).     |

### Wrap strategies

`--wrap` controls how the raw identifier returned by the winning source
is turned into a UUID. Every strategy is deterministic: the same raw
input always produces the same UUID.

| Value          | Behaviour                                                 | When to pick it                                                                                       |
| -------------- | --------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| `v5`           | UUID v5 (SHA-1) under this crate's private namespace      | **Default.** Rehashes even pre-UUID sources (DMI, `IOPlatformUUID`, `MachineGuid`) so two tools sharing the same raw source cannot collide. |
| `v3`           | UUID v3 (MD5) under the nil namespace                     | Wire-compat with legacy Go tooling that used `uuid.NewMD5(uuid.Nil, raw)`. Interop only — RFC 9562 recommends v5 over v3. |
| `passthrough`  | Parse the raw value directly as a UUID, with no hashing   | You want the source's own UUID (e.g. `product_uuid`, pod UID) to survive unchanged so it matches another agent on the same host. Errors if the raw value isn't a parseable UUID. |

Pick `v5` unless you have a concrete interop requirement. The library
API (`Resolver::with_wrap`) also exposes `UuidV5With(ns)` for callers
who need v5 hashing under a caller-supplied namespace — the CLI does
not expose that variant because it takes a non-stringly-typed namespace
UUID.

### Subcommands

| Subcommand | Purpose                                                          |
| ---------- | ---------------------------------------------------------------- |
| `resolve`  | Resolve and print the host identity (default when omitted).      |
| `audit`    | Walk every source in the chain and report each outcome.          |
| `sources`  | List every source identifier compiled into this binary.          |

## Features

| Feature     | Default | Pulls in                                                         |
| ----------- | ------- | ---------------------------------------------------------------- |
| `container` | yes     | `host-identity/container`                                        |
| `network`   | yes     | `ureq` + every cloud feature of `host-identity` + `host-identity/k8s` |

Without `network`, `--network` at runtime produces an error directing the
user to rebuild with the feature.

## Packaging

A man page is committed at `man/host-identity.1` (plus one page per subcommand)
at the workspace root. Packagers should install it to
`$PREFIX/share/man/man1/host-identity.1`. See the top-level [`README.md`][pkg]
for the full install recipe. The pages are regenerated from the `clap`
metadata with `cargo xtask`.

[pkg]: https://github.com/dekobon/host-identity#packaging

## See also

- [`host-identity`] — the library.

[`host-identity`]: https://crates.io/crates/host-identity

## License

Dual-licensed under Apache-2.0 or MIT at your option.
