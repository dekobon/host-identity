# `hostid` — a CLI for [`host-identity`]

Resolve a stable, collision-resistant host UUID across platforms, container
runtimes, cloud providers, and Kubernetes. This crate ships the `hostid`
binary as a thin wrapper over the [`host-identity`] library; use the library
directly if you need to embed the same logic in another program.

## Install

```bash
cargo install host-identity-cli
```

This gives you the `hostid` executable on your `PATH`. Default features
enable both local (machine-id, DMI, container) and network (cloud metadata,
Kubernetes) sources. To build a strictly local binary:

```bash
cargo install host-identity-cli --no-default-features --features container
```

## Usage

```bash
# Print the host UUID (default chain, local sources only).
hostid

# Same, but include cloud-metadata and Kubernetes sources.
hostid resolve --network

# Walk every source without short-circuiting — useful for diagnostics.
hostid audit

# List every source identifier compiled into this binary.
hostid sources

# Build a custom chain from source identifiers.
hostid resolve --sources env-override,machine-id,dmi

# Machine-readable output.
hostid resolve --format json
hostid audit --format json
```

### Flags

| Flag                 | Values                          | Default | Notes                                                      |
| -------------------- | ------------------------------- | ------- | ---------------------------------------------------------- |
| `--format`           | `plain`, `summary`, `json`      | `plain` | `summary` prints `source:uuid`; `plain` prints only UUID.  |
| `--wrap`             | `v5`, `v3`, `passthrough`       | `v5`    | UUID derivation strategy. `v3` matches legacy Go tooling.  |
| `--sources <ids>`    | comma-separated source IDs      | *(unset)* | Build a custom chain; see `hostid sources`.              |
| `--network`          | *(flag)*                        | off     | Adds cloud / k8s sources (requires `network` feature).     |

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

A man page is committed at `man/hostid.1` (plus one page per subcommand)
at the workspace root. Packagers should install it to
`$PREFIX/share/man/man1/hostid.1`. See the top-level [`README.md`][pkg]
for the full install recipe. The pages are regenerated from the `clap`
metadata with `cargo xtask`.

[pkg]: https://github.com/dekobon/host-identity#packaging

## See also

- [`host-identity`] — the library.

[`host-identity`]: https://crates.io/crates/host-identity

## License

Dual-licensed under Apache-2.0 or MIT at your option.
