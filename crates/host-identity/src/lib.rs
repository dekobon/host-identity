//! Stable, collision-resistant host identity.
//!
//! Many agents and telemetry pipelines need a *stable* identifier for the
//! host they run on: one that survives restarts and upgrades but distinguishes
//! two otherwise-identical hosts. The obvious source on modern Linux is
//! `/etc/machine-id`, but relying on it alone is unreliable: cloned VMs share
//! IDs, LXC guests often inherit the host's ID, minimal container images
//! have no file at all, and systemd writes the literal string `uninitialized`
//! during early boot.
//!
//! `host-identity` exposes every known-good identity source as a composable
//! [`Source`] implementation. Consumers can either take the default
//! platform-appropriate chain or mix and match sources in any order to match
//! their own policy.
//!
//! # The common case
//!
//! ```no_run
//! // Local-only default chain: env override → platform sources →
//! // (container ID when applicable). No network calls.
//! let id = host_identity::resolve()?;
//! println!("{id}");
//! # Ok::<(), host_identity::Error>(())
//! ```
//!
//! When you want cloud-metadata endpoints in the chain as well, use
//! [`resolve_with_transport`] with your HTTP client of choice. That
//! chain is strictly richer than the local default: it includes every
//! cloud and Kubernetes source the feature set enabled, ordered so that
//! per-pod identity outranks per-container outranks per-instance
//! outranks per-host software state.
//!
//! # Identifier-based chains (config-driven)
//!
//! For operator-facing config files, build a chain from a list of
//! short string identifiers:
//!
//! ```
//! use host_identity::ids::{resolver_from_ids, source_ids};
//!
//! // Equivalent to:
//! //   Resolver::new()
//! //       .push(EnvOverride::new("HOST_IDENTITY"))
//! //       .push(MachineIdFile::default())
//! //       .push(DmiProductUuid::default())
//! let resolver = resolver_from_ids([
//!     source_ids::ENV_OVERRIDE,
//!     source_ids::MACHINE_ID,
//!     source_ids::DMI,
//! ]).unwrap();
//! ```
//!
//! See [`ids`] for the full list of identifiers and the
//! `_with_transport` variant for cloud sources.
//!
//! # Auditing every source
//!
//! [`resolve_all`] and [`resolve_all_with_transport`] walk the same
//! chains without short-circuiting and return one [`ResolveOutcome`]
//! per source. Use them when you want to see what every source would
//! produce — operator diagnostics, debugging, cross-validation. To
//! audit a caller-chosen subset, build the resolver with exactly those
//! sources and call [`Resolver::resolve_all`]:
//!
//! ```no_run
//! use host_identity::Resolver;
//! use host_identity::sources::{MachineIdFile, DmiProductUuid};
//!
//! let outcomes = Resolver::new()
//!     .push(MachineIdFile::default())
//!     .push(DmiProductUuid::default())
//!     .resolve_all();
//! for outcome in outcomes {
//!     println!("{:?} → {:?}", outcome.source(), outcome.host_id());
//! }
//! ```
//!
//! # Mixing and matching
//!
//! Every built-in source is a public type that implements [`Source`]. Chain
//! them in any order, add your own, and pick the wrap strategy:
//!
//! ```no_run
//! use host_identity::{Resolver, Wrap};
//! use host_identity::sources::{EnvOverride, FileOverride, MachineIdFile, DmiProductUuid};
//!
//! let id = Resolver::new()
//!     .push(EnvOverride::new("MY_APP_HOST_ID"))
//!     .push(DmiProductUuid::default())    // SMBIOS first — stable across OS reinstalls
//!     .push(MachineIdFile::default())
//!     .push(FileOverride::new("/etc/my-app/host-id"))
//!     .with_wrap(Wrap::UuidV5Namespaced)
//!     .resolve()?;
//! # Ok::<(), host_identity::Error>(())
//! ```
//!
//! # Starting from the defaults
//!
//! [`Resolver::with_defaults`] pre-loads the platform's default chain. Use
//! [`Resolver::prepend`] to add higher-priority sources (e.g. your own
//! override) or [`Resolver::push`] to add fallbacks:
//!
//! ```no_run
//! use host_identity::Resolver;
//! use host_identity::sources::FileOverride;
//!
//! let id = Resolver::with_defaults()
//!     .push(FileOverride::new("/etc/host-identity"))  // last-resort fallback
//!     .resolve()?;
//! # Ok::<(), host_identity::Error>(())
//! ```
//!
//! # Custom sources
//!
//! Implement [`Source`] directly, or wrap a closure with
//! [`sources::FnSource`]:
//!
//! ```no_run
//! use host_identity::sources::FnSource;
//! use host_identity::{Resolver, SourceKind};
//!
//! let custom = FnSource::new(SourceKind::custom("hsm"), || {
//!     // Read from an HSM, a custom config file, an in-house identity
//!     // service, etc.
//!     Ok(Some("h-0abc1234".to_owned()))
//! });
//!
//! let id = Resolver::new().push(custom).resolve()?;
//! # Ok::<(), host_identity::Error>(())
//! ```
//!
//! # Cloud-metadata sources
//!
//! Each major cloud provider has a dedicated source behind an opt-in
//! feature flag. Sources are generic over a caller-supplied
//! [`transport::HttpTransport`]; the crate ships no HTTP client of its
//! own.
//!
//! | Source                              | Feature        |
//! | ----------------------------------- | -------------- |
//! | [`sources::AwsImds`]                | `aws`          |
//! | [`sources::GcpMetadata`]            | `gcp`          |
//! | [`sources::AzureImds`]              | `azure`        |
//! | [`sources::DigitalOceanMetadata`]   | `digitalocean` |
//! | [`sources::HetznerMetadata`]        | `hetzner`      |
//! | [`sources::OciMetadata`]            | `oci`          |
//!
//! Implement [`transport::HttpTransport`] for your HTTP client of choice
//! (adapters are ~10 lines against `reqwest`, `ureq`, `hyper`, etc.), or
//! pass a closure — a blanket impl accepts any
//! `Fn(http::Request<Vec<u8>>) -> Result<http::Response<Vec<u8>>, E>`.
//! For providers the crate doesn't ship, implement
//! [`sources::CloudEndpoint`] on a zero-sized type and alias
//! [`sources::CloudMetadata`].
//!
//! # Kubernetes sources
//!
//! Feature `k8s` (no new dependencies) exposes
//! [`sources::KubernetesPodUid`] (from `/proc/self/mountinfo`),
//! [`sources::KubernetesServiceAccount`] (from the mounted SA secret),
//! and [`sources::KubernetesDownwardApi`] (any file projected by a
//! `downwardAPI` volume).

#![warn(missing_docs)]
#![forbid(unsafe_code)]

mod error;
mod hostid;
pub mod ids;
mod resolver;
mod source;
pub mod sources;
#[cfg(feature = "_transport")]
pub mod transport;
mod wrap;

pub use error::Error;
pub use hostid::{HostId, HostIdSummary, ResolveOutcome};
#[cfg(feature = "_transport")]
pub use ids::resolver_from_ids_with_transport;
pub use ids::{UnknownSourceError, resolver_from_ids};
pub use resolver::Resolver;
pub use source::{Probe, Source, SourceKind};
pub use wrap::{DEFAULT_NAMESPACE, Wrap};

/// Resolve a stable host identity using the default chain for this platform.
///
/// Equivalent to `Resolver::with_defaults().resolve()`. This chain is
/// strictly local — no source makes network calls. Reach for
/// [`resolve_with_transport`] when you want cloud-metadata endpoints in
/// the chain, or [`Resolver`] when you need to reorder sources, add
/// custom ones, or choose a different wrap strategy.
pub fn resolve() -> Result<HostId, Error> {
    Resolver::with_defaults().resolve()
}

/// Resolve a stable host identity using the default chain plus every
/// cloud-metadata and Kubernetes source enabled at compile time.
///
/// Equivalent to `Resolver::with_network_defaults(transport).resolve()`.
/// Requires a caller-supplied [`transport::HttpTransport`] — the crate
/// ships no HTTP client. The transport must be `Clone + 'static`; wrap
/// your client in `Arc` if needed.
///
/// Available only when at least one cloud feature is enabled (`aws`,
/// `gcp`, `azure`, `digitalocean`, `hetzner`, `oci`). See
/// [`Resolver::with_network_defaults`] for the full chain order.
#[cfg(feature = "_transport")]
pub fn resolve_with_transport<T>(transport: T) -> Result<HostId, Error>
where
    T: transport::HttpTransport + Clone + 'static,
{
    Resolver::with_network_defaults(transport).resolve()
}

/// Walk the default local chain without short-circuiting and return every
/// source's [`ResolveOutcome`] in order.
///
/// Complement to [`resolve`]: same chain, same wrap strategy — but every
/// source is consulted regardless of whether earlier sources already
/// succeeded or failed. Useful for auditing which sources on this host
/// agree (or disagree) and for operator tooling that wants to present
/// the full picture.
///
/// To audit a caller-chosen subset instead of the defaults, build the
/// resolver directly: `Resolver::new().push(...).push(...).resolve_all()`.
#[must_use]
pub fn resolve_all() -> Vec<ResolveOutcome> {
    Resolver::with_defaults().resolve_all()
}

/// Walk the network-enabled default chain without short-circuiting and
/// return every source's [`ResolveOutcome`] in order.
///
/// Complement to [`resolve_with_transport`]: same chain, same wrap
/// strategy — but every source is consulted. Useful for auditing which
/// cloud or Kubernetes metadata endpoints are reachable from this host,
/// and what each would yield.
///
/// Available only when at least one cloud feature is enabled.
#[cfg(feature = "_transport")]
#[must_use]
pub fn resolve_all_with_transport<T>(transport: T) -> Vec<ResolveOutcome>
where
    T: transport::HttpTransport + Clone + 'static,
{
    Resolver::with_network_defaults(transport).resolve_all()
}
