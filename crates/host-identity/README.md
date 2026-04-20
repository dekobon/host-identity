# host-identity

A Rust crate that produces a *stable*, *distinct* identifier for the host
your program is running on — and stays correct in the edge cases most
host-id libraries quietly get wrong.

## The problem

Every observability agent, license client, fleet management system, and
telemetry pipeline needs an answer to the same question: **which host is
this?** It sounds trivial. In practice it is one of the most reliably
broken bits of infrastructure plumbing in modern deployments.

The obvious answer on Linux is `/etc/machine-id`: a systemd-managed file
written once at first boot, persistent across reboots, and unique per
machine. Except it often isn't. Here is a representative sample of ways
that assumption breaks in real fleets:

- **Cloned virtual machines share a machine-id.** systemd's contract is
  that `/etc/machine-id` must be absent or empty in a VM template so the
  first boot regenerates it. That preparation step is skipped often
  enough that Proxmox, VMware, and most provisioning pipelines carry
  open bug reports for it. When it is skipped, every clone reports the
  same ID and the management plane merges their telemetry onto one
  record.
- **LXC containers inherit the host's machine-id.** Unless the
  administrator explicitly clears it, every LXC guest on a given host
  reports the host's ID. This is tracked as a known bug against
  Proxmox's LXC integration.
- **Docker doesn't mount `/etc/machine-id` into containers.** The file
  is simply absent in a default Docker container, so naive agents fall
  back to minting a random UUID on startup. Every container restart
  orphans the previous record in the management plane and creates a
  brand-new one.
- **Red Hat container images bind-mount the host's machine-id.** RHEL
  has shipped containers this way since 2015 as a workaround for
  unrelated yum tooling. Every container on a given host reports the
  host's ID, and pod-level telemetry from many pods collapses onto one
  record.
- **systemd writes the literal string `uninitialized` during early
  boot.** An agent that starts in that window reads the sentinel,
  hashes it like any other value, and emits the same UUID on every
  host in that state. Nothing in the shape of the string reveals it
  as a sentinel; a hash is a hash.
- **Minimal images don't have the file at all.** busybox, distroless,
  and some Alpine configurations ship without `/etc/machine-id`.
  Agents that treat "file missing" as "generate a random UUID" end up
  churning a fresh identifier on every restart, producing instance
  churn that corrupts long-range trends.
- **There is rarely an operator override.** An administrator who
  inherits a fleet with known duplicate machine-ids — from a hypervisor
  bug, a cloned template, a bind-mounted host file — has no escape
  hatch inside the agent. They cannot say "use this ID instead" without
  rebuilding from source or patching files the distribution owns.

None of these are exotic. All of them happen routinely, and any of them
is enough to corrupt a telemetry pipeline, misdirect config pushes, or
collapse a week of metrics onto the wrong record.

## What this crate does

`host-identity` treats "host identity" as a layered problem rather than a
single file read. It models each probe as an **identity source** — a
single mechanism the crate can consult to learn who the host is,
whether a file, a registry key, a `sysctl`, an HTTP metadata endpoint,
or a caller-supplied closure — and composes identity sources into an
ordered chain. It provides:

**A platform-appropriate chain of identity sources.** On Linux that
means `/etc/machine-id`, then `/var/lib/dbus/machine-id`, then the
SMBIOS product UUID from `/sys/class/dmi/id/product_uuid`. On macOS,
`IOPlatformUUID`. On Windows, the registry's `MachineGuid`. On FreeBSD,
`/etc/hostid` then the SMBIOS value via `kenv`. On NetBSD and OpenBSD,
`sysctl kern.hostid`. On illumos and Solaris, `hostid(1)`. The first
identity source that produces a usable value wins.

**Container awareness.** When the crate detects a container runtime on
Linux, it extracts the container ID from `/proc/self/mountinfo` — the
same way Docker, Kubernetes, and CRI-O agents do — and uses that as the
identity. Containers get their own identity rather than inheriting the
host's, and the patterns match what existing tooling already produces so
IDs flow through unchanged. LXC and LXD guests are handled separately by
`LxcId`: their container name is read from `/proc/self/cgroup` or
`/proc/self/mountinfo` and salted with `/etc/machine-id` so two
identically-named containers on different hosts cannot collide.

**Identity scope is explicit.** Sources produce identities at
different scopes: per-pod (`KubernetesPodUid`), per-container
(`ContainerId`, `LxcId`), per-instance (cloud metadata, SMBIOS) and
per-host-OS (`machine-id`, registry `MachineGuid`). Every cloud
metadata source returns the *host's* instance ID even when the
caller is running inside a container on that instance — twenty
containers on one EC2 host all read the same `AwsImds` value. The
default chains place per-pod above per-container above per-instance
above per-host-OS so a process in a container wins on its own ID
and never falls through to a host-scope source. If you build a
custom chain, keep that ordering — see
[`docs/algorithm.md` → "Identity scope"](../../docs/algorithm.md#identity-scope-what-host-means-per-source)
for the full rationale.

**Sentinel detection.** The `uninitialized` string and empty files are
rejected at the source layer, not hashed and passed through. A host
caught in early boot falls through to the next source in the chain
instead of colliding with every other host in the same state.

**An operator override.** Both an environment variable (`HOST_IDENTITY`)
and a file path can supply the identity directly, and are checked first
by the default chain. Fleets with known-duplicate machine-ids can fix
the problem without recompiling.

**Deterministic wrapping, no random fallback.** Raw identifiers are
wrapped as UUID v5 under a crate-owned namespace so the same input
always produces the same UUID. If every source in the chain produces
nothing, `resolve()` returns an error rather than minting a random
UUID — callers decide whether to treat that as fatal, log it, or apply
their own recovery.

**Composable identity sources.** Each identity source is a public type
implementing the `Source` trait. Use the default chain with one function
call, or build your own in whatever order you like. Built-in opt-in
identity sources cover the common cloud metadata services (AWS IMDSv2,
GCP, Azure, DigitalOcean, Hetzner, OCI) and Kubernetes (pod UID,
service-account namespace, downward-API projected files). For anything
else — an HSM read, a custom config path, an in-house identity service —
wrap a closure in `FnSource`.

**Bring-your-own HTTP client.** Network-backed identity sources are
generic over an `HttpTransport` trait. The crate ships no HTTP client:
adapt your existing `reqwest`, `ureq`, `hyper`, or tower stack with ~10
lines, or pass a closure. Sync-only by design so any async runtime works
via `block_on`.

Nothing here is magic. These are the fallbacks the `gopsutil` library
already applies on Go, the checks the Linux kernel community has
documented for a decade, and the override knobs operators keep
reinventing because no library ships them. `host-identity` puts them in
one place with a composable API so you don't have to reassemble them
from scratch.

For the full step-by-step algorithm — every source's probe semantics,
which outcomes advance the chain, which short-circuit it, how the raw
identifier becomes a UUID, and what each error variant means — see
[`docs/algorithm.md`](../../docs/algorithm.md).

## Default algorithm (local-only)

```rust
let id = host_identity::resolve()?;
```

Equivalent to `Resolver::with_defaults().resolve()`. Tries the
environment override, the container source (Linux, with the `container`
feature), and the platform's native sources in recommended order.
**No source in this chain makes a network call.**

## Default algorithm with network sources

```rust
let id = host_identity::resolve_with_transport(my_http_client)?;
```

Equivalent to `Resolver::with_network_defaults(t).resolve()`. Strictly
a superset of the local default — starts with the same env override,
then inserts every cloud-metadata and Kubernetes source the crate was
compiled with, ordered so that **per-pod identity outranks
per-container outranks per-instance outranks per-host software state**:

1. `HOST_IDENTITY` env override.
2. Kubernetes pod UID (feature `k8s`).
3. Container ID (feature `container`, Linux only).
4. Cloud sources for every enabled cloud feature in declaration order:
   `aws`, `gcp`, `azure`, `digitalocean`, `hetzner`, `oci`. Each
   short-circuits to `Ok(None)` when its endpoint is unreachable so the
   chain falls through to the next source.
5. Platform-native local sources (machine-id, DMI, `ioreg`, registry, …).
6. Kubernetes service-account namespace (feature `k8s`) as a coarse
   last-ditch fallback.

Requires a caller-supplied `HttpTransport`. The transport must be
`Clone + 'static` (each cloud source owns its own handle); wrap your
client in `Arc` if the underlying type isn't cheaply cloneable.

## Auditing every source (no short-circuit)

`resolve()` and `resolve_with_transport()` stop at the first usable
source. For operator tooling, diagnostics, or cross-validation across
sources, walk the whole chain and get back one outcome per source:

```rust
// Every default source, every outcome, in chain order:
for outcome in host_identity::resolve_all() {
    println!("{:?} → {:?}", outcome.source(), outcome.host_id());
}

// Caller-chosen subset: use the same builder that feeds resolve().
use host_identity::Resolver;
use host_identity::sources::{MachineIdFile, DmiProductUuid};

let outcomes = Resolver::new()
    .push(MachineIdFile::default())
    .push(DmiProductUuid::default())
    .resolve_all();
```

`resolve_all()` returns `Vec<ResolveOutcome>`. Each entry is either
`Found(HostId)`, `Skipped(SourceKind)` (the source had nothing to
offer), or `Errored(SourceKind, Error)` (the source produced a hard
error that in a short-circuiting `resolve()` would have aborted the
chain). Every source is consulted regardless of what earlier sources
returned. `resolve_all_with_transport(transport)` mirrors this for the
network-enabled chain.

## Mix and match

Every source is a public type implementing `Source`. Build your own
chain in whatever order you want:

```rust
use host_identity::{Resolver, Wrap};
use host_identity::sources::{
    EnvOverride, FileOverride, DmiProductUuid, MachineIdFile,
};

let id = Resolver::new()
    .push(EnvOverride::new("MY_APP_HOST_ID"))
    .push(DmiProductUuid::default())       // SMBIOS first: survives OS reinstall
    .push(MachineIdFile::default())
    .push(FileOverride::new("/etc/my-app/host-id"))
    .with_wrap(Wrap::UuidV5Namespaced)
    .resolve()?;
```

Start from the platform defaults and extend on either end:

```rust
let id = Resolver::with_defaults()
    .prepend(my_high_priority_source)           // checked before defaults
    .push(FileOverride::new("/etc/fallback"))   // last-resort fallback
    .resolve()?;
```

## Custom sources

Implement `Source` directly, or wrap a closure:

```rust
use host_identity::sources::FnSource;
use host_identity::{Resolver, SourceKind};

let hsm = FnSource::new(SourceKind::custom("hsm"), || {
    // Read from an HSM, a custom config file, an in-house identity service…
    Ok(Some(read_from_hsm()?))
});

let id = Resolver::new().push(hsm).resolve()?;
# fn read_from_hsm() -> Result<String, host_identity::Error> { Ok("x".into()) }
```

## Cloud-metadata sources

Each major cloud provider has a dedicated source behind an opt-in
feature flag. Sources are generic over a caller-supplied
`HttpTransport`:

```rust
use host_identity::Resolver;
use host_identity::sources::AwsImds;
use host_identity::transport::HttpTransport;

// Adapt whichever HTTP client your project already uses.
struct MyClient { /* ureq::Agent, reqwest::blocking::Client, … */ }

impl HttpTransport for MyClient {
    type Error = MyError;
    fn send(
        &self,
        request: http::Request<Vec<u8>>,
    ) -> Result<http::Response<Vec<u8>>, Self::Error> {
        // Translate http::Request ↔ your client's request type,
        // make the call, translate the response back.
        # unimplemented!()
    }
}

let id = Resolver::new()
    .push(AwsImds::new(MyClient { /* … */ }))
    .resolve()?;
```

Prefer to skip the trait impl? The blanket impl accepts any closure:

```rust
let transport = |req: http::Request<Vec<u8>>| my_client.call(req);
let source = host_identity::sources::GcpMetadata::new(transport);
```

Available cloud sources (each behind its named feature):

| Source                   | Feature         | Endpoint                                              |
| ------------------------ | --------------- | ----------------------------------------------------- |
| `AwsImds<T>`             | `aws`           | IMDSv2 (PUT token → GET instance identity document)   |
| `GcpMetadata<T>`         | `gcp`           | `metadata.google.internal/computeMetadata/v1/…`       |
| `AzureImds<T>`           | `azure`         | `169.254.169.254/metadata/instance/…?format=text`     |
| `DigitalOceanMetadata<T>`| `digitalocean`  | `169.254.169.254/metadata/v1/id`                      |
| `HetznerMetadata<T>`     | `hetzner`       | `169.254.169.254/hetzner/v1/metadata/instance-id`     |
| `OciMetadata<T>`         | `oci`           | `169.254.169.254/opc/v2/instance/id`                  |

Transport or HTTP-level failures (connection refused, TLS errors, non-2xx
responses) all map to `Ok(None)`, so the resolver falls through to the
next source when the host clearly isn't on that provider. A 2xx response
with an unparseable body is a hard error.

### Adding a provider

Providers that follow the one-GET plaintext-response pattern can be
added in ~25 lines by implementing `CloudEndpoint`:

```rust
use host_identity::{SourceKind, sources::{CloudEndpoint, CloudMetadata}};

pub type VultrMetadata<T> = CloudMetadata<VultrEndpoint, T>;

pub struct VultrEndpoint;

impl CloudEndpoint for VultrEndpoint {
    const DEBUG_NAME: &'static str = "VultrMetadata";
    const DEFAULT_BASE_URL: &'static str = "http://169.254.169.254";
    const PATH: &'static str = "/v1/instance-id";
    const KIND: SourceKind = SourceKind::custom("vultr-metadata");

    fn headers() -> &'static [(&'static str, &'static str)] { &[] }
}
```

## Kubernetes sources

Feature `k8s` (no new dependencies):

```rust
use host_identity::Resolver;
use host_identity::sources::{
    KubernetesPodUid, KubernetesServiceAccount, KubernetesDownwardApi,
};

let id = Resolver::new()
    .push(KubernetesPodUid::default())              // /proc/self/mountinfo
    .push(KubernetesDownwardApi::with_label(
        "/etc/podinfo/uid",
        "pod-uid",                                  // custom log label
    ))
    .push(KubernetesServiceAccount::default())      // namespace (fallback)
    .resolve()?;
```

`KubernetesPodUid` extracts the UID from cgroup paths in
`/proc/self/mountinfo`, handling both cgroup v1 (`/kubepods/pod<uid>/…`)
and the systemd cgroup driver (`kubepods-pod<uid>.slice`) form.
`KubernetesDownwardApi` reads an arbitrary file the pod spec projects
via a `downwardAPI` volume; use `with_label` when chaining several so
each probe's provenance is distinguishable in telemetry.

## Platforms and sources

| Platform         | Default native sources                                 |
| ---------------- | ------------------------------------------------------ |
| Linux            | `MachineIdFile`, `DbusMachineIdFile`, `DmiProductUuid` (opt-in: `LinuxHostIdFile` for `/etc/hostid`) |
| macOS            | `IoPlatformUuid` (via `ioreg`)                         |
| Windows          | `WindowsMachineGuid` (registry)                        |
| FreeBSD          | `FreeBsdHostIdFile`, `KenvSmbios`                      |
| NetBSD / OpenBSD | `SysctlKernHostId`                                     |
| illumos / Solaris| `IllumosHostId` (`hostid(1)`)                          |

Cross-platform sources: `EnvOverride`, `FileOverride`, `FnSource`, and
`ContainerId` (which probes only on Linux; the type is available
everywhere). Platform-specific source types compile on every target and
no-op (`Ok(None)`) off their native OS, so a portable chain needs no
`cfg` gates at the call site.

Opt-in sources: cloud metadata (AWS, GCP, Azure, DigitalOcean, Hetzner,
OCI) and Kubernetes (pod UID, service-account namespace, downward-API
projected files). See the sections below.

## Clone-collision risk by source

Not every source is equally robust against cloning. Some are written once
at install or first-boot and then copied along with the disk image; others
are regenerated per instance by the hypervisor or runtime. The table
below breaks down each built-in source and the realistic risk that two
distinct hosts end up reporting the same value.

| Source               | Reads                                          | Network call?                   | Clone-collision risk                     | Why                                                                                                                                                                              |
| -------------------- | ---------------------------------------------- | ------------------------------- | ---------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `MachineIdFile`      | `/etc/machine-id`                              | No                              | **High**                                 | Shared by cloned VMs whose templates were not cleared per systemd's contract. Bind-mounted from the host into many Red Hat container images. Inherited by LXC guests by default. |
| `DbusMachineIdFile`  | `/var/lib/dbus/machine-id`                     | No                              | **High**                                 | Usually a symlink to `/etc/machine-id`; inherits every risk above. When present as a separate file it was generated at the same first-boot moment and is cloned alongside.       |
| `DmiProductUuid`     | `/sys/class/dmi/id/product_uuid`               | No                              | **Low on bare metal, low–medium in VMs** | SMBIOS system UUID. Set by the OEM on physical hardware. Regenerated per VM by VMware, Hyper-V, Proxmox, and libvirt clone-mode. Can still collide when a VM is deployed by copying disk files outside the hypervisor's clone tooling. Requires root to read on most distributions. |
| `ContainerId`        | `/proc/self/mountinfo`                         | No                              | **None**                                 | The runtime assigns a fresh ID to every container. A restarted container is a new container, which is the correct semantics at the container layer.                              |
| `LxcId`              | `/proc/self/cgroup`, `/proc/self/mountinfo`    | No                              | **Low**                                  | LXC/LXD container name salted with `/etc/machine-id`. Two hosts running a container with the same name still get distinct IDs because the salt differs; only an operator explicitly cloning both hosts and both containers together can collide.                                                                                        |
| `IoPlatformUuid`     | `ioreg IOPlatformExpertDevice`                 | No                              | **Low on Apple hardware, medium in VMs** | Set at factory time on physical Macs. In VM products (Parallels, UTM, VMware Fusion) it may be derived from the VM config; duplicating the config file without randomizing it produces collisions. |
| `WindowsMachineGuid` | `HKLM\…\Cryptography\MachineGuid`              | No                              | **High**                                 | Written by the installer and never regenerated. Every Windows image cloned without `sysprep /generalize` shares this GUID. A well-known class of bug in Windows fleet management. |
| `FreeBsdHostIdFile`  | `/etc/hostid`                                  | No                              | **High**                                 | Written at first boot and persists. VM templates that aren't cleared before cloning produce collisions the same way `/etc/machine-id` does.                                       |
| `LinuxHostIdFile`    | `/etc/hostid` (glibc 4-byte binary)            | No                              | **High**                                 | Written once (by `sethostid(2)`, `zgenhostid`, or the image build) and cloned with the disk. Opt-in only — not part of either default chain — because the file is absent on most stock Linux distros. |
| `KenvSmbios`         | `kenv smbios.system.uuid`                      | No                              | **Low on bare metal, low–medium in VMs** | Same SMBIOS analysis as `DmiProductUuid`.                                                                                                                                       |
| `SysctlKernHostId`   | `sysctl kern.hostid`                           | No                              | **High**                                 | Often `0` on fresh installs; once set, persists through the host's configuration and is cloned alongside it.                                                                     |
| `IllumosHostId`      | `hostid(1)`                                    | No                              | **Medium–high**                          | Historically derived from licensed hardware. On modern illumos it is frequently seeded from `/etc/hostid` or zone config, both of which clone with the image.                    |
| `EnvOverride`        | environment variable                           | No                              | **Operator-controlled**                  | As reliable as the fleet's deployment policy. Useful as an escape hatch when upstream sources are known to collide.                                                              |
| `FileOverride`       | caller-supplied file                           | No                              | **Operator-controlled**                  | Same as `EnvOverride`, but survives environment resets. Path can live in a per-instance volume to guarantee uniqueness.                                                          |
| `FnSource`           | caller closure                                 | **Depends on closure**          | **Depends on closure**                   | HSM reads, in-house identity services, custom config lookups. The caller owns both the I/O behaviour and the clone-risk profile. For cloud-metadata sources prefer the dedicated `AwsImds`, `GcpMetadata`, etc. — they share one `HttpTransport` trait and keep protocol details out of the closure. |
| Cloud sources        | provider metadata endpoint (via `HttpTransport`) | **Yes — one GET (AWS: PUT + GET)** | **None — regenerated per instance by the provider** | Every major cloud provider assigns a distinct instance ID when the VM is created. These sources resist clone collisions by construction but require the host to reach the provider's link-local endpoint at resolution time. |
| K8s sources          | pod cgroup path or mounted secret file         | No                              | **None (pod UID) / namespace-level (service account)** | `KubernetesPodUid` is unique per pod. `KubernetesServiceAccount` yields the namespace only, so every pod in the same namespace collides at that layer — use as a coarse fallback below a per-pod source. `KubernetesDownwardApi` inherits whatever uniqueness the pod spec projects. |

### Choosing a chain that resists cloning

Two practical rules of thumb:

1. **Prefer hypervisor- or hardware-regenerated sources over software
   state.** `DmiProductUuid`, `KenvSmbios`, and cloud instance IDs (via
   `FnSource`) are reset by the infrastructure that creates the VM.
   `MachineIdFile`, `WindowsMachineGuid`, and `SysctlKernHostId` are
   software state that travels with the image. The former category
   resists routine cloning; the latter does not.
2. **Place an override at the top of any chain that serves a fleet with
   known cloning issues.** `EnvOverride` and `FileOverride` let an
   operator correct individual hosts without patching the image or
   rebuilding the agent. The default chain checks `HOST_IDENTITY` for
   exactly this reason.

A resolver that defends against the common failure modes ends up looking
roughly like this:

```rust
use host_identity::Resolver;
use host_identity::sources::{EnvOverride, DmiProductUuid, MachineIdFile};

let id = Resolver::new()
    .push(EnvOverride::new("HOST_IDENTITY"))   // operator escape hatch
    .push(DmiProductUuid::default())           // hypervisor-regenerated
    .push(MachineIdFile::default())            // software-state fallback
    .resolve()?;
```

The default chain applies the same idea across every platform the crate
supports.

## Wrap strategies

How a raw identifier becomes a UUID, selected with `Resolver::with_wrap`:

| Strategy              | Behaviour                                           |
| --------------------- | --------------------------------------------------- |
| `UuidV5Namespaced`    | Default. UUID v5 under the crate's namespace.       |
| `UuidV5With(ns)`      | UUID v5 under a caller-supplied namespace.          |
| `UuidV3Nil`           | UUID v3 under the nil namespace (legacy Go compat). |
| `Passthrough`         | Parse the raw value directly as a UUID.             |

## Features

- `container` *(default)* — enables the Linux container source
  (`ContainerId`). No extra dependencies.
- `aws` — `AwsImds<T>` (IMDSv2). Pulls in the `http` crate.
- `gcp` — `GcpMetadata<T>`. Pulls in the `http` crate.
- `azure` — `AzureImds<T>`. Pulls in the `http` crate.
- `digitalocean` — `DigitalOceanMetadata<T>`. Pulls in the `http` crate.
- `hetzner` — `HetznerMetadata<T>`. Pulls in the `http` crate.
- `oci` — `OciMetadata<T>`. Pulls in the `http` crate.
- `k8s` — `KubernetesPodUid`, `KubernetesServiceAccount`,
  `KubernetesDownwardApi`. No extra dependencies.

Turning on any cloud feature also brings in the `HttpTransport` trait
and the `CloudEndpoint` extension point for consumer-defined providers.
The crate ships no HTTP client of its own — picking one (sync, async,
TLS backend, connection pool) is the consumer's decision.

**Naming convention**: Cargo features use **compact** names
(`digitalocean`, `hetzner`) because Cargo feature names can't contain
hyphens. The identifier strings used by `SourceKind::as_str`,
`SourceKind::from_id`, and `ids::source_ids` are **hyphenated**
(`digital-ocean-metadata`, `hetzner-metadata`). If you're writing a
config file, use the hyphenated form; if you're writing a `Cargo.toml`,
use the compact form.

## Guarantees

- **Deterministic**: a given raw input always maps to the same UUID.
- **No random fallback**: `resolve()` returns `Error::NoSource` rather
  than minting a per-restart UUID.
- **Sentinel-aware**: `uninitialized` and empty files are rejected.
- **No `unsafe`**: the crate sets `unsafe_code = "forbid"` at the root.

## License

Licensed under either of Apache License, Version 2.0 or MIT license at
your option.
