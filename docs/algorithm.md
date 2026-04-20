# Default algorithm

This document describes exactly what happens when a caller invokes
`resolve()` or `resolve_with_transport(t)` — the ordering of identity
sources, what each identity source returns in each scenario, which
outcomes advance the chain and which short-circuit it, and how the raw
identifier becomes the final UUID.

An **identity source** is a single mechanism the crate can probe for a
host identifier (a file, registry key, `sysctl`, metadata endpoint, or
caller-supplied closure); the resolver walks an ordered chain of them.

It is written as a reference, not a tutorial. For the "what and why"
pitch, see `README.md`; for per-type API detail, see the rustdoc on
`Resolver`, `Source`, each source struct, and `Wrap`.

## Entry points

The crate exposes four canonical resolution functions — two
short-circuiting (stop at first success) and two full-walk (return
every source's outcome). All four delegate to a `Resolver` builder
with a different pre-populated chain.

| Entry point                                 | Stops at first success? | Builder method            |
| ------------------------------------------- | ----------------------- | ------------------------- |
| `resolve()`                                 | Yes                     | `Resolver::resolve`       |
| `resolve_with_transport(transport)`         | Yes                     | `Resolver::resolve`       |
| `resolve_all()`                             | No — walks all sources  | `Resolver::resolve_all`   |
| `resolve_all_with_transport(transport)`     | No — walks all sources  | `Resolver::resolve_all`   |

`resolve()` is the local-only entry point — **no source in its chain
makes a network call**. It's present on every build.

`resolve_with_transport` is the network-enabled entry point. It exists
only when at least one cloud feature is compiled in (`aws`, `gcp`,
`azure`, `digitalocean`, `hetzner`, `oci`). It requires a caller-supplied
`HttpTransport` — the crate ships no HTTP client. The transport must be
`Clone + 'static`: each cloud source owns its own handle, so a client
that isn't cheaply cloneable should be wrapped in `Arc` before use.

Both chains end in the same wrap stage; only the list of sources
differs.

## Identity scope: what "host" means per source

Different identity sources identify things at different scopes. A
process running on a cloud VM inside a container inside Kubernetes
has at least four distinct identities available to it, and they are
not interchangeable. Which one the resolver picks is entirely a
function of chain order — which is why the default chains are ordered
the way they are.

| Scope          | Answers "who is…"                        | Built-in sources                                                                     |
| -------------- | ---------------------------------------- | ------------------------------------------------------------------------------------ |
| Per-pod        | this Kubernetes pod?                     | `KubernetesPodUid`, `KubernetesDownwardApi` (when projecting `metadata.uid`)         |
| Per-container  | this container runtime instance?         | `ContainerId`, `LxcId`                                                               |
| Per-instance   | this virtual machine (hypervisor scope)? | `AwsImds`, `GcpMetadata`, `AzureImds`, `DigitalOceanMetadata`, `HetznerMetadata`, `OciMetadata`, `DmiProductUuid`, `KenvSmbios`, `IoPlatformUuid` |
| Per-host-OS    | this OS install / boot identity?         | `MachineIdFile`, `DbusMachineIdFile`, `WindowsMachineGuid`, `FreeBsdHostIdFile`, `SysctlKernHostId`, `IllumosHostId` |
| Per-namespace  | which Kubernetes namespace?              | `KubernetesServiceAccount`                                                           |
| Caller-pinned  | whatever the operator says.              | `EnvOverride`, `FileOverride`, `FnSource`                                            |
| Wrapper        | inherits the inner source's scope.       | `AppSpecific<S>` — per-app derivation of any other source; scope == `S`'s scope     |

### The trap: host-scope sources from inside a container

Every per-instance and per-host-OS source returns the **host's**
identity, even when the caller is running inside a container on that
host. That is by design — AWS IMDS, `/etc/machine-id`, SMBIOS — none
of them know or care that a container exists. The operational
consequence:

> On a single EC2 instance running twenty Docker containers, every
> container reading `AwsImds` alone sees the same instance ID. Their
> telemetry collapses onto one row. The cloud source didn't fail; it
> correctly reported what the caller asked for — *the host's
> identity*. The bug is asking a host-scope source for container
> identity.

This is the exact failure the crate is designed to prevent.
Per-container and per-pod sources must sit **above** per-instance and
per-host-OS sources in the chain; a process in a container then wins
on its container ID and never consults the host-scope sources below
it. A process on a bare VM (no container) falls through the empty
container-scope probes and wins on a per-instance source.

The default chains enforce this:

- `default_chain()` places `ContainerId` above the per-host-OS Linux
  sources. Bare-metal and VM hosts with no container runtime fall
  through `ContainerId`'s `Ok(None)` and land on `MachineIdFile` /
  `DmiProductUuid`.
- `network_default_chain(t)` places `KubernetesPodUid` above
  `ContainerId` above the per-instance cloud sources above the
  per-host-OS sources, so the most specific available layer wins.

### Caveats when building a custom chain

- **Don't put cloud sources above `ContainerId` / `KubernetesPodUid`.**
  A container running on EC2 will resolve to the instance ID, and
  every sibling container on the same instance will collide on it.
- **Bind-mounted `/etc/machine-id` inherits host scope.** Red Hat
  container images bind-mount the host's `machine-id` by default;
  `MachineIdFile` reads the bind-mounted file and returns the host's
  identity, not the container's. `ContainerId` sitting above it is
  what keeps containers distinct.
- **`EnvOverride` is caller-controlled scope.** Whoever sets
  `HOST_IDENTITY` picks the scope. Setting it in a host-level systemd
  unit gives host scope to every container that inherits the
  environment; setting it in a pod spec gives per-pod scope. Pick
  intentionally.
- **`HostId::in_container()` is provenance, not scope.** It reports
  *whether the resolver ran inside a container*, not whether the
  returned ID is container-scoped. A container resolving under
  `EnvOverride` with a host-scope value still sees
  `in_container() == true`.

## The resolver loop (short-circuiting)

`Resolver::resolve()` walks the chain in order. For each source it
calls `probe()` and branches on the result:

| `probe()` returns     | Loop behaviour                                              |
| --------------------- | ----------------------------------------------------------- |
| `Ok(Some(probe))`     | **Stop**. Wrap the probe's raw value and return `HostId`.   |
| `Ok(None)`            | Continue to the next source.                                |
| `Err(err)`            | **Stop immediately**. Return `err` to the caller.           |

Two things are worth emphasising:

1. **`Err` from any source short-circuits the entire chain.** Lower-priority
   sources are never consulted. A source must return `Ok(None)` — not
   `Err` — to mean "I don't apply here; please try the next one." This
   is why every built-in source treats "not on this OS," "file absent,"
   "endpoint unreachable," and similar conditions as `Ok(None)` rather
   than errors. `Err` is reserved for situations the caller would want
   to know about (corrupt file, sentinel value, I/O failure on a file
   that *should* have been there).

2. **The chain is linear.** There is no parallelism, no retry, no
   timeout layer. Each source is consulted exactly once, in order, and
   the first success wins. Retry and timeout belong in the transport a
   consumer plugs into the cloud sources — there is nothing to retry
   for a missing `/etc/machine-id`.

If every source in the chain returns `Ok(None)`, the resolver returns
`Error::NoSource { tried }`. `tried` is a comma-separated list of the
source labels that were consulted, so operators can diagnose "nothing
matched" without reading the source code.

## The resolver loop (full walk)

`Resolver::resolve_all()` walks the same chain using the same
per-source probe logic but never short-circuits. Each source is
consulted exactly once, and its outcome is recorded as a
[`ResolveOutcome`]:

| `probe()` returns     | Recorded outcome                                            |
| --------------------- | ----------------------------------------------------------- |
| `Ok(Some(probe))`     | `ResolveOutcome::Found(HostId)` after wrapping the raw.     |
| `Ok(None)`            | `ResolveOutcome::Skipped(SourceKind)`.                      |
| `Err(err)`            | `ResolveOutcome::Errored(SourceKind, Error)`.               |

The wrap step can itself fail (only under `Wrap::Passthrough` with a
non-UUID raw value); when it does, the outcome is
`Errored(kind, Error::Malformed)` and the walk continues.

The return type is `Vec<ResolveOutcome>`, in chain order, one entry
per source. Call `.host_id()` on each outcome to pull out any `HostId`
that was produced, or match on the variant directly for richer
diagnostics. `resolve_all()` always succeeds — the per-source errors
are data, not a control-flow signal.

### Caller-chosen subsets

`resolve_all` is a method on `Resolver`, so specifying an exact set of
sources to audit uses the same builder as short-circuiting resolution:

```rust
let outcomes = Resolver::new()
    .push(MachineIdFile::default())
    .push(DmiProductUuid::default())
    .push(some_custom_source)
    .resolve_all();
```

No source is added that the caller didn't `push` / `prepend` —
`Resolver::new()` starts empty. `with_defaults()` /
`with_network_defaults(t)` are shortcuts for the two pre-built chains;
`with_sources(iter)` swaps in a caller-built list wholesale.

### When to use each

- **`resolve()` / `resolve_with_transport(t)`** — normal production use.
  You want one identity; you want it fast; you want errors that indicate
  real problems to propagate.
- **`resolve_all()` / `resolve_all_with_transport(t)`** — operator
  tooling, diagnostics, test harnesses, and cross-validation. You want
  to see every source's outcome and keep going regardless of individual
  failures. Equivalent to running `resolve()` once per source, except
  it runs in chain order, shares one container-detection call, and
  returns a single ordered vector.

## Local-only chain (`default_chain`)

The order is identical on every platform; the platform-gated blocks
compile to empty on other OSes. On a given host only the blocks for
that OS contribute sources.

```text
1. EnvOverride("HOST_IDENTITY")      — every platform
2. ContainerId                       — Linux + feature "container"
3. LxcId                             — Linux + feature "container"
4. MachineIdFile                     — Linux
5. DbusMachineIdFile                 — Linux
6. DmiProductUuid                    — Linux
7. IoPlatformUuid                    — macOS
8. WindowsMachineGuid                — Windows
9. FreeBsdHostIdFile                 — FreeBSD
10. KenvSmbios                       — FreeBSD
11. SysctlKernHostId                 — NetBSD / OpenBSD
12. IllumosHostId                    — illumos / Solaris
```

### Rationale

- **Operator override first.** Any fleet with known-duplicate machine-ids
  — from a cloned VM template, a bind-mounted host file, an LXC guest
  inheriting its host — needs an escape hatch that doesn't require
  recompiling. `EnvOverride` provides it.
- **Container identity before host identity on Linux.** If the process
  is running in a container, the container is more specific than its
  host. Two containers on the same host should not collapse to one ID.
- **Native sources last, in platform-specific order.** Each platform's
  native list prefers the source most likely to survive cloning. See
  `README.md` for the clone-collision analysis.

## Network-enabled chain (`network_default_chain`)

```text
1. EnvOverride("HOST_IDENTITY")       — every platform
2. KubernetesPodUid                   — feature "k8s"
3. ContainerId                        — Linux + feature "container"
4. LxcId                              — Linux + feature "container"
5. AwsImds<T>                         — feature "aws"
6. GcpMetadata<T>                     — feature "gcp"
7. AzureImds<T>                       — feature "azure"
8. DigitalOceanMetadata<T>            — feature "digitalocean"
9. HetznerMetadata<T>                 — feature "hetzner"
10. OciMetadata<T>                    — feature "oci"
11. MachineIdFile / DbusMachineIdFile / DmiProductUuid — Linux
12. IoPlatformUuid                    — macOS
13. WindowsMachineGuid                — Windows
14. FreeBsdHostIdFile / KenvSmbios    — FreeBSD
15. SysctlKernHostId                  — NetBSD / OpenBSD
16. IllumosHostId                     — illumos / Solaris
17. KubernetesServiceAccount          — feature "k8s"
```

### Ordering principle

Per-pod identity outranks per-container outranks per-instance outranks
per-host software state, with operator override pinned to the top and a
coarse service-account fallback pinned to the bottom:

- **KubernetesPodUid (2)** identifies the pod. Two pods on one node get
  distinct IDs.
- **ContainerId (3)** identifies the container runtime's view. For a
  standalone Docker container (no Kubernetes) this is the right layer.
- **Cloud metadata (4–9)** identifies the VM instance. On a bare VM —
  no pod, no container — this is the right layer.
- **Platform-native sources (10–15)** are software state on the host.
  Always available, but most collision-prone of the per-host options.
- **KubernetesServiceAccount (16)** yields only the pod's namespace, so
  every pod in the namespace collides. Useful as a last-ditch fallback
  when everything else failed (unlikely but possible on exotic
  runtimes).

### Feature gating

Any step whose feature isn't enabled is simply absent from the chain —
the compiler strips its `push(...)` call. Similarly, platform-specific
steps compile to nothing off their native OS. A build with
`default-features = false, features = ["aws", "k8s"]` on Linux produces
the chain:

```text
EnvOverride → KubernetesPodUid → AwsImds<T> → MachineIdFile → DbusMachineIdFile → DmiProductUuid → KubernetesServiceAccount
```

`container` was not enabled, so step 3 is missing. No other cloud
features were enabled, so steps 5–9 are missing. On macOS the same
feature set would produce:

```text
EnvOverride → KubernetesPodUid → AwsImds<T> → IoPlatformUuid → KubernetesServiceAccount
```

`KubernetesPodUid` and `KubernetesServiceAccount` remain in the chain
off Linux but their `probe()` returns `Ok(None)` because
`/proc/self/mountinfo` and the service-account mount don't exist there.

## Per-source probe semantics

This is the full contract each source honours. Every source treats "I
don't apply here" as `Ok(None)`; `Err` is reserved for conditions the
caller needs to know about.

### `EnvOverride`

- Read the named environment variable.
- `Err` from `std::env::var` → `Ok(None)` (variable unset).
- Empty or whitespace-only value → `Ok(None)` (after `normalize`).
- `uninitialized` sentinel → `Ok(None)` (env overrides are caller
  input; rejecting the sentinel there is silent because the caller can
  simply set the variable again).
- Otherwise → `Ok(Some(trimmed))`.

### `FileOverride`

- Read the configured path.
- `NotFound` → `Ok(None)`.
- Empty / whitespace-only / sentinel → `Ok(None)` (via `normalize`).
- Any other I/O error → `Err(Error::Io { path, source })`.
- Otherwise → `Ok(Some(trimmed))`.

Note the asymmetry with `MachineIdFile` below: `FileOverride` does not
swallow `PermissionDenied`. A file the operator chose to point at must
be readable; if it isn't, that's a configuration error worth
propagating.

### `FnSource`

- Call the closure. Return whatever it returns (after running its
  output through `normalize` if it was `Some`).

### `ContainerId`

- Read `/proc/self/mountinfo`.
- Any read failure → `Ok(None)` (not on Linux, procfs not mounted, etc.).
- No line matches any of the five container-ID patterns (Docker,
  containerd, CRI-O scope unit, Podman, sandboxed containerd) → `Ok(None)`.
- Match → `Ok(Some(<64-hex container id>))`.

### `LxcId`

- Read `/etc/machine-id`; `NotFound`, empty, or the `uninitialized`
  sentinel → `Ok(None)` (fall through silently; the primary
  `MachineIdFile` below will surface the sentinel loudly if it is the
  authoritative path).
- Scan `/proc/self/cgroup` for one of: `/lxc.payload.<name>` or
  `/lxc.monitor.<name>` (substring match — the literal dots make these
  markers unambiguous) or the legacy `/lxc/<name>` (prefix match only,
  to avoid false-matching `/usr/share/lxc/templates/...`).
- If cgroup yields nothing, scan `/proc/self/mountinfo` with the same
  markers — modern LXD resets the container's cgroup view via
  cgroup-namespacing, and the name is only recoverable from bind-mount
  source paths that leak through mountinfo.
- No match → `Ok(None)`.
- Match → `Ok(Some("lxc:<machine_id>:<name>"))`. Salting with
  `machine-id` makes the raw value unique across hosts before the
  `Wrap` stage hashes it; two different hosts running a container
  with the same name cannot collide.

Placed immediately below `ContainerId` in both default chains so a
Docker-in-LXC nested deployment resolves to the innermost Docker ID —
`ContainerId` fires first and short-circuits.

### `MachineIdFile`, `DbusMachineIdFile`, `DmiProductUuid`

All three share the same `read_id_file` implementation and differ only
in their default path and `SourceKind`.

- `NotFound` → `Ok(None)`.
- `PermissionDenied` → `Ok(None)` with a `log::debug!` entry. DMI needs
  root on most distributions; this branch keeps an unprivileged process
  from erroring out of the whole chain.
- `uninitialized` sentinel (systemd's early-boot marker) →
  `Err(Error::Uninitialized { path })`. This is the one case where the
  file exists, is readable, and has content, but the content would
  produce a fleet-wide collision if hashed. The caller should know.
- Empty / whitespace-only → `Ok(None)`.
- Any other I/O error → `Err(Error::Io { path, source })`.
- `MachineIdFile` and `DbusMachineIdFile` additionally reject
  known-duplicate machine-id values (Whonix's anti-fingerprinting
  constant, hex values baked into widely-pulled container images such
  as the official `oraclelinux:8`/`oraclelinux:9` images, and any
  all-same-nibble 32-hex value such as all-zero) → `Ok(None)` with a
  `log::debug!` entry, so a host that inherits a shared image value
  falls through to the next source rather than producing a fleet-wide
  non-unique identity. The list is deliberately narrow: only cited,
  publicly-documented shared values. A false positive here drops a
  legitimate host, so a missing entry is strictly preferable to an
  over-broad rule.
- `DmiProductUuid` additionally rejects known-garbage SMBIOS values
  (all-zero, all-F, all-same-nibble, and a curated list of vendor
  placeholders such as `03000200-0400-0500-0006-000700080009`) →
  `Ok(None)` with a `log::debug!` entry, so a box shipping a
  motherboard-default UUID falls through to the next source rather
  than producing a fleet-wide non-unique identity.
- Otherwise → `Ok(Some(trimmed))`.

### `IoPlatformUuid` (macOS)

- Run `/usr/sbin/ioreg -rd1 -c IOPlatformExpertDevice`.
- Spawn failure → `Err(Error::Platform("ioreg: ..."))`.
- Non-zero exit → `Ok(None)` with a `log::debug!` entry.
- No `IOPlatformUUID` line in output → `Ok(None)`.
- Otherwise → `Ok(Some(<uuid>))`.

### `WindowsMachineGuid`

- Query `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid`.
- Missing key / missing value → `Ok(None)`.
- Other registry failure → `Err`.
- Otherwise → `Ok(Some(<guid>))`.

### `FreeBsdHostIdFile`, `KenvSmbios`, `SysctlKernHostId`, `IllumosHostId`

- Same shape as the Linux file/command sources: missing file or failing
  command → `Ok(None)`; sentinel / empty → `Ok(None)`; usable →
  `Ok(Some)`; unexpected I/O → `Err(Error::Io)` or `Err(Error::Platform)`.

### `LinuxHostIdFile`

Opt-in: not part of `default_chain` or `network_default_chain`. Push it
explicitly only on hosts where `/etc/hostid` is known to be populated
(OpenZFS hosts, minimal non-systemd images, Red Hat containers that
bind-mount `machine-id` but not `hostid`).

- `NotFound` → `Ok(None)`.
- `PermissionDenied` → `Ok(None)` with a `log::debug!` entry.
- File size ≠ 4 bytes → `Ok(None)` with a `log::debug!` entry
  (defensive: sheared reads, FreeBSD-style text UUID mistakenly placed
  on Linux).
- Decoded `u32::from_ne_bytes(bytes)` value of `0x00000000` or
  `0xffffffff` → `Ok(None)` with a `log::debug!` entry (unset or
  known-garbage sentinels).
- Any other I/O error → `Err(Error::Io { path, source })`.
- Otherwise → `Ok(Some(<8-digit lowercase hex>))`, matching
  `hostid(1)` output.

This source reads the file directly rather than calling `gethostid(3)`;
glibc's fallback fabricates a value from `gethostname()` → IPv4 lookup
when the file is absent, and that value is neither stable nor unique.

### `KubernetesPodUid`

- Read `/proc/self/mountinfo`.
- Any read failure → `Ok(None)`.
- No `pod` marker preceded by a cgroup separator (start-of-word, `/`,
  `-`) followed by a 36-character canonical UUID → `Ok(None)`.
- Match → `Ok(Some(<lowercased dashed uuid>))`. Underscore-separated
  form (systemd cgroup driver) is normalised to dashes; any uppercase
  hex is normalised to lowercase so mixed-case variants hash
  consistently.

### `KubernetesServiceAccount`

- Read `/var/run/secrets/kubernetes.io/serviceaccount/namespace` (or a
  caller-supplied path via `::at`).
- `NotFound` → `Ok(None)` (not in a pod).
- Empty → `Ok(None)`.
- Other I/O error → `Err(Error::Io { path, source })`.
- Otherwise → `Ok(Some(<namespace>))`. Note that this is the namespace,
  not a per-pod identifier; every pod in the namespace yields the same
  value. Use below a per-pod source.

### `KubernetesDownwardApi`

- Read the caller-supplied path.
- `NotFound` → `Ok(None)`.
- Empty → `Ok(None)`.
- Other I/O error → `Err(Error::Io { path, source })`.
- Otherwise → `Ok(Some(<content>))`. `with_label("…")` labels the probe
  as `SourceKind::Custom("…")` so multiple downward-API sources stay
  distinguishable in logs.

### `AwsImds<T>`

- PUT to `{base_url}/latest/api/token` with the TTL header.
- PUT transport error → `Ok(None)` (not on EC2 / reachable).
- Token response non-2xx → `Ok(None)` (IMDSv1-only host, IMDS
  misconfigured).
- Token body not UTF-8 → `Ok(None)`.
- GET `{base_url}/latest/dynamic/instance-identity/document` with the
  token header.
- GET transport error → `Ok(None)`.
- Document non-2xx → `Ok(None)`.
- Document body not UTF-8 → `Ok(None)`.
- Document 2xx but no `instanceId` field → `Err(Error::Platform("..."))`.
  This is the "contract violation" case: IMDS responded, so we're on
  EC2, but the document shape isn't what AWS documents. The caller
  should see that.
- Otherwise → `Ok(Some(<instance-id>))`.

### `GcpMetadata<T>`, `AzureImds<T>`, `DigitalOceanMetadata<T>`, `HetznerMetadata<T>`, `OciMetadata<T>`

All five share the same `CloudMetadata<E, T>` implementation and differ
only in the endpoint descriptor (URL, headers, `SourceKind`).

- GET `{base_url}{path}` with provider-specific headers.
- Transport error → `Ok(None)`.
- Non-2xx → `Ok(None)` with a `log::debug!` entry.
- Non-UTF-8 body → `Ok(None)`.
- Empty / whitespace-only body → `Ok(None)` (after `normalize`).
- Otherwise → `Ok(Some(<trimmed-body>))`.

None of the plaintext providers error on an empty response body —
unlike AWS, none of them return a structured document whose schema
this crate needs to validate.

## The wrap stage

Once a source returns `Ok(Some(probe))`, the resolver converts the raw
string into a `uuid::Uuid` using the configured `Wrap` strategy. The
default is `Wrap::UuidV5Namespaced`.

| Strategy                | Action                                                            | Can fail? |
| ----------------------- | ----------------------------------------------------------------- | --------- |
| `UuidV5Namespaced`      | UUID v5 (SHA-1) under the crate's fixed namespace.                | No        |
| `UuidV5With(ns)`        | UUID v5 under the caller-supplied namespace.                      | No        |
| `UuidV3Nil`             | UUID v3 (MD5) under the nil namespace (legacy Go-compat).         | No        |
| `Passthrough`           | Parse the raw value directly as a UUID.                           | Yes       |

If `Passthrough` is chosen and the raw value isn't a valid UUID, the
resolver returns `Error::Malformed { source_kind, reason }`. Every
other strategy always succeeds; any bytes at all will hash.

### Why hash at all when the source already yields a UUID?

`DmiProductUuid`, `IoPlatformUuid`, `WindowsMachineGuid`, and
`KenvSmbios` produce canonical UUID strings. The default still rehashes
them under `UuidV5Namespaced`. That's deliberate: two independent tools
reading the same raw source (for example, two observability agents both
hashing `/etc/machine-id` into UUID v5) produce colliding IDs if they
share a namespace. The crate's namespace is fixed and randomly chosen,
not shared with any other tool, so the output doesn't collide with an
identifier derived by anyone else — even from the same raw input. Use
`Passthrough` only when you explicitly want the source's own UUID to
survive unchanged.

## The `HostId.in_container` bit

`HostId::in_container()` reports whether the host was running inside a
container at resolution time. On Linux it's computed independently of
the source chain: `/.dockerenv` exists, or `/proc/1/cgroup` contains
one of the runtime markers (`docker`, `kubepods`, `containerd`,
`podman`, `lxc`, `crio`). On every other platform it's always `false`.

The bit is provenance, not selection: it tells the caller that the
resolved ID was produced inside a container without the caller having
to compare `source()` to `SourceKind::Container`. This lets the bit
stay meaningful even when a higher-priority source (an `EnvOverride`,
for instance) won the chain.

## Error modes

`Error` has five variants; `resolve()` can return any of them:

| Variant                         | Raised by                                                                                   |
| ------------------------------- | ------------------------------------------------------------------------------------------- |
| `Error::NoSource { tried }`     | Every source in the chain returned `Ok(None)`.                                              |
| `Error::Uninitialized { path }` | A machine-id-shaped file contained the `uninitialized` sentinel.                            |
| `Error::Io { path, source }`    | An I/O error other than `NotFound` / `PermissionDenied` on a local source.                  |
| `Error::Malformed { source_kind, reason }` | A source returned a value that couldn't be turned into a UUID (only possible under `Wrap::Passthrough`, or a source-specific contract violation). |
| `Error::Platform(reason)`       | A platform tool or cloud endpoint failed in a way the caller should know about (e.g. `ioreg` spawn failed, AWS IMDS returned a document without `instanceId`). |

Callers who treat "no identity available" as recoverable should match
on `Error::NoSource`. Every other variant indicates a condition that
likely won't resolve without operator intervention.

## External references

Each source below follows a third-party convention. Links are to the
authoritative document the implementation tracks; per-source rustdoc in
`crates/host-identity/src/sources/*.rs` cites the same references.

| Source                                      | Authoritative document                                                                                                                                             |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `MachineIdFile`                             | systemd [`machine-id(5)`](https://www.freedesktop.org/software/systemd/man/latest/machine-id.html)                                                                 |
| `DbusMachineIdFile`                         | [D-Bus Specification — UUIDs](https://dbus.freedesktop.org/doc/dbus-specification.html#uuids)                                                                      |
| `DmiProductUuid`                            | kernel [`sysfs-firmware-dmi-tables`](https://www.kernel.org/doc/Documentation/ABI/testing/sysfs-firmware-dmi-tables) · [DMTF SMBIOS DSP0134](https://www.dmtf.org/dsp/DSP0134) |
| `IoPlatformUuid` (macOS)                    | [`ioreg(8)`](https://keith.github.io/xcode-man-pages/ioreg.8.html) · [Apple IOKit](https://developer.apple.com/documentation/iokit)                                |
| `WindowsMachineGuid`                        | [CNG Registry Keys](https://learn.microsoft.com/en-us/windows/win32/seccng/cng-registry-keys) · [Windows Registry](https://learn.microsoft.com/en-us/windows/win32/sysinfo/registry) |
| `FreeBsdHostIdFile`                         | FreeBSD [`hostid(1)`](https://man.freebsd.org/cgi/man.cgi?query=hostid&sektion=1), [`gethostid(3)`](https://man.freebsd.org/cgi/man.cgi?query=gethostid&sektion=3) |
| `LinuxHostIdFile`                           | GNU coreutils [`hostid(1)`](https://www.gnu.org/software/coreutils/hostid) · Linux [`gethostid(3)`](https://man7.org/linux/man-pages/man3/gethostid.3.html) · [`sethostid(2)`](https://man7.org/linux/man-pages/man2/sethostid.2.html) |
| `KenvSmbios`                                | FreeBSD [`kenv(1)`](https://man.freebsd.org/cgi/man.cgi?query=kenv&sektion=1) · [DMTF SMBIOS DSP0134](https://www.dmtf.org/dsp/DSP0134)                             |
| `SysctlKernHostId`                          | NetBSD [`sysctl(7)`](https://man.netbsd.org/sysctl.7) · OpenBSD [`sysctl(8)`](https://man.openbsd.org/sysctl.8), [`gethostid(3)`](https://man.openbsd.org/gethostid.3) |
| `IllumosHostId`                             | illumos [`hostid(1)`](https://illumos.org/man/1/hostid) · [`sysinfo(2)`](https://illumos.org/man/2/sysinfo)                                                        |
| `ContainerId`                               | [OCI Runtime Specification](https://github.com/opencontainers/runtime-spec/blob/main/spec.md) · [`proc_pid_mountinfo(5)`](https://man7.org/linux/man-pages/man5/proc_pid_mountinfo.5.html) · [`cgroups(7)`](https://man7.org/linux/man-pages/man7/cgroups.7.html) |
| `LxcId`                                     | [`lxc.container.conf(5)`](https://linuxcontainers.org/lxc/manpages/man5/lxc.container.conf.5.html) · [`cgroups(7)`](https://man7.org/linux/man-pages/man7/cgroups.7.html) · [`proc_pid_mountinfo(5)`](https://man7.org/linux/man-pages/man5/proc_pid_mountinfo.5.html) |
| `KubernetesPodUid`                          | [kubelet cgroup drivers](https://kubernetes.io/docs/concepts/architecture/cgroups/)                                                                                |
| `KubernetesServiceAccount`                  | [Kubernetes: Configure service accounts for pods](https://kubernetes.io/docs/tasks/configure-pod-container/configure-service-account/)                             |
| `KubernetesDownwardApi`                     | [Kubernetes: Downward API volume files](https://kubernetes.io/docs/tasks/inject-data-application/downward-api-volume-expose-pod-information/)                      |
| `AwsImds<T>`                                | [AWS: Use IMDSv2](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/configuring-instance-metadata-service.html) · [Instance identity documents](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/instance-identity-documents.html) |
| `GcpMetadata<T>`                            | [Compute Engine: About VM metadata](https://cloud.google.com/compute/docs/metadata/overview)                                                                       |
| `AzureImds<T>`                              | [Azure Instance Metadata Service](https://learn.microsoft.com/en-us/azure/virtual-machines/instance-metadata-service)                                              |
| `DigitalOceanMetadata<T>`                   | [DigitalOcean: Droplet metadata API](https://docs.digitalocean.com/reference/api/metadata-api/)                                                                    |
| `HetznerMetadata<T>`                        | [Hetzner Cloud: Server metadata](https://docs.hetzner.cloud/#server-metadata)                                                                                      |
| `OciMetadata<T>`                            | [OCI: Getting instance metadata](https://docs.oracle.com/en-us/iaas/Content/Compute/Tasks/gettingmetadata.htm)                                                     |
| `Wrap::UuidV5*`, `Wrap::UuidV3Nil`          | [RFC 9562 § 5.3 (v3)](https://datatracker.ietf.org/doc/html/rfc9562#name-uuid-version-3) · [§ 5.5 (v5)](https://datatracker.ietf.org/doc/html/rfc9562#name-uuid-version-5) (obsoletes [RFC 4122](https://datatracker.ietf.org/doc/html/rfc4122)) |

## What this algorithm doesn't do

- **No random fallback.** If the chain produces nothing, `resolve()`
  returns `Error::NoSource`. The caller decides whether to treat that
  as fatal, log it, or mint their own per-run UUID. The crate will not
  quietly create an identifier that varies between restarts.
- **No caching.** Each `resolve()` call walks the chain from scratch.
  Callers that resolve frequently should cache the result; the crate
  does not second-guess how long an ID stays valid.
- **No persistence.** The crate reads identity sources; it never writes
  to them. An operator intending to pin a host's ID should configure
  `FileOverride`, `EnvOverride`, or the system's native machine-id
  tooling — not ask the crate to persist.
- **No retry or timeout.** Cloud sources short-circuit to `Ok(None)` on
  any transport error; they do not retry, back off, or time out. Those
  concerns belong in the `HttpTransport` the caller supplies.
