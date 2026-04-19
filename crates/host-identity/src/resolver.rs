//! The [`Resolver`] — a configurable chain of [`Source`]s.

use crate::error::Error;
use crate::hostid::{HostId, ResolveOutcome};
use crate::source::{Probe, Source, SourceKind};
use crate::sources;
use crate::wrap::Wrap;

const EMPTY_RAW_REASON: &str = "raw identifier is empty";

/// A composable chain of identity sources.
///
/// Use [`Resolver::with_defaults`] for the platform-appropriate default
/// chain, or [`Resolver::new`] to start empty and build your own order with
/// [`Resolver::push`] / [`Resolver::prepend`].
pub struct Resolver {
    sources: Vec<Box<dyn Source>>,
    wrap: Wrap,
}

impl Resolver {
    /// Start with an empty chain. No sources are tried until you add some.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
            wrap: Wrap::default(),
        }
    }

    /// Start with the default chain for the current platform.
    ///
    /// The chain begins with the `HOST_IDENTITY` environment variable
    /// override, then — on Linux, when the `container` feature is on —
    /// inserts the container source ahead of the host-level sources so
    /// containers get their own identity, then walks the platform's native
    /// sources in recommended order. See [`sources::default_chain`] for the
    /// exact contents on each OS.
    ///
    /// This chain is strictly local: no source makes network calls.
    #[must_use]
    pub fn with_defaults() -> Self {
        Self {
            sources: sources::default_chain(),
            wrap: Wrap::default(),
        }
    }

    /// Default chain plus every cloud-metadata and Kubernetes source the
    /// consumer's feature set enabled.
    ///
    /// Requires a caller-supplied [`crate::transport::HttpTransport`]; the
    /// crate ships no HTTP client. The transport must be `Clone + 'static`
    /// because each cloud source owns its own handle — wrap a non-cloneable
    /// client in `Arc` if necessary.
    ///
    /// Source order (each step is only present when its feature is on):
    ///
    /// 1. `HOST_IDENTITY` env override.
    /// 2. Kubernetes pod UID (feature `k8s`; returns `Ok(None)` off Linux).
    /// 3. Container ID from `/proc/self/mountinfo` (feature `container`;
    ///    Linux only).
    /// 4. Cloud-metadata sources for every enabled cloud feature, in the
    ///    declaration order: `aws`, `gcp`, `azure`, `digitalocean`,
    ///    `hetzner`, `oci`. Each returns `Ok(None)` when its endpoint is
    ///    unreachable so the chain falls through to the next.
    /// 5. Platform-native local sources (machine-id, DMI, registry, …).
    /// 6. Kubernetes service-account namespace (feature `k8s`) as a coarse
    ///    last-ditch fallback below every per-host source.
    ///
    /// The ordering keeps per-pod identity above per-container above
    /// per-instance above per-host software state.
    #[cfg(feature = "_transport")]
    #[must_use]
    pub fn with_network_defaults<T>(transport: T) -> Self
    where
        T: crate::transport::HttpTransport + Clone + 'static,
    {
        Self {
            sources: sources::network_default_chain(transport),
            wrap: Wrap::default(),
        }
    }

    /// Append a source to the end of the chain (lowest priority).
    #[must_use]
    pub fn push<S: Source + 'static>(mut self, source: S) -> Self {
        self.sources.push(Box::new(source));
        self
    }

    /// Append an already-boxed source. Use when you have `Box<dyn Source>`
    /// already — for example when building a chain from runtime input via
    /// [`crate::ids::resolver_from_ids`].
    #[must_use]
    pub fn push_boxed(mut self, source: Box<dyn Source>) -> Self {
        self.sources.push(source);
        self
    }

    /// Prepend a source to the front of the chain (highest priority).
    ///
    /// O(n) in the existing chain length — each call shifts every other
    /// source. For a chain assembled from many prepends, build the full
    /// list first and pass it to [`Resolver::with_sources`] instead.
    #[must_use]
    pub fn prepend<S: Source + 'static>(mut self, source: S) -> Self {
        self.sources.insert(0, Box::new(source));
        self
    }

    /// Replace the entire chain. All items must be the same concrete
    /// `Source` type; for heterogeneous chains use
    /// [`Resolver::with_boxed_sources`].
    #[must_use]
    pub fn with_sources<I, S>(self, sources: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Source + 'static,
    {
        self.with_boxed_sources(sources.into_iter().map(|s| Box::new(s) as Box<dyn Source>))
    }

    /// Replace the entire chain with an already-boxed, heterogeneous
    /// list. Use when you have sources of different concrete types —
    /// `with_sources` requires a single concrete type for all items, so
    /// a mixed chain has to be boxed first.
    ///
    /// ```
    /// use host_identity::{Resolver, Source};
    /// use host_identity::sources::{EnvOverride, FnSource};
    /// # use host_identity::SourceKind;
    ///
    /// let chain: Vec<Box<dyn Source>> = vec![
    ///     Box::new(EnvOverride::new("HOST_IDENTITY")),
    ///     Box::new(FnSource::new(SourceKind::custom("x"), || Ok(None))),
    /// ];
    /// let resolver = Resolver::new().with_boxed_sources(chain);
    /// # let _ = resolver;
    /// ```
    #[must_use]
    pub fn with_boxed_sources<I>(mut self, sources: I) -> Self
    where
        I: IntoIterator<Item = Box<dyn Source>>,
    {
        self.sources = sources.into_iter().collect();
        self
    }

    /// Set the UUID-wrapping strategy applied to the raw identifier.
    ///
    /// Defaults to [`Wrap::UuidV5Namespaced`].
    #[must_use]
    pub fn with_wrap(mut self, wrap: Wrap) -> Self {
        self.wrap = wrap;
        self
    }

    /// Inspect the configured chain — useful for tests, diagnostics, and
    /// logging the resolver shape at startup.
    #[must_use]
    pub fn source_kinds(&self) -> Vec<SourceKind> {
        self.source_kinds_iter().collect()
    }

    /// Non-allocating view of the chain's source kinds, in order.
    ///
    /// Use when you want to iterate without materialising a `Vec` —
    /// e.g. constructing a log line, or checking whether a specific
    /// kind is present. The returned iterator borrows `self` and must
    /// not outlive the resolver.
    #[allow(
        clippy::redundant_closure_for_method_calls,
        reason = "the suggested `Source::kind` reference requires explicit deref through `Box<dyn Source>` and reads worse than the closure"
    )]
    pub fn source_kinds_iter(&self) -> impl Iterator<Item = SourceKind> + '_ {
        self.sources.iter().map(|s| s.kind())
    }

    /// Walk the chain and return the first successful identity.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NoSource`] if every source returned `Ok(None)`,
    /// or [`Error::Malformed`] if the selected [`Wrap`] is
    /// [`Wrap::Passthrough`] and the raw value is not a valid UUID. Other
    /// [`Error`] variants bubble up from the source that produced them
    /// (permission denied, sentinel value, platform-tool failure).
    pub fn resolve(&self) -> Result<HostId, Error> {
        for source in &self.sources {
            if let Some(probe) = source.probe()? {
                return self.probe_to_host_id(probe, detected_container());
            }
        }
        let tried = self
            .source_kinds_iter()
            .map(SourceKind::as_str)
            .collect::<Vec<_>>()
            .join(",");
        Err(Error::NoSource { tried })
    }

    /// Walk the entire chain without short-circuiting and return one
    /// [`ResolveOutcome`] per source.
    ///
    /// Complements [`Resolver::resolve`]: the chain, wrap strategy, and
    /// container-detection logic are identical — only the stopping
    /// behaviour differs. Every source is consulted exactly once, in
    /// chain order, and neither a success nor an error stops the walk.
    ///
    /// Use this to audit what each source would produce — operator
    /// diagnostics, debugging, or test harnesses that want to confirm
    /// that several sources agree. For normal resolution use
    /// [`Resolver::resolve`], which stops at the first usable source.
    ///
    /// To run a caller-chosen subset of sources, build the resolver with
    /// exactly those sources — the same builder that feeds `resolve()`:
    ///
    /// ```no_run
    /// use host_identity::Resolver;
    /// use host_identity::sources::{MachineIdFile, DmiProductUuid};
    ///
    /// let report = Resolver::new()
    ///     .push(MachineIdFile::default())
    ///     .push(DmiProductUuid::default())
    ///     .resolve_all();
    /// for outcome in report {
    ///     println!("{:?} → {:?}", outcome.source(), outcome.host_id());
    /// }
    /// ```
    #[must_use]
    pub fn resolve_all(&self) -> Vec<ResolveOutcome> {
        let in_container = detected_container();
        self.sources
            .iter()
            .map(|source| {
                let kind = source.kind();
                match source.probe() {
                    Ok(Some(probe)) => self.outcome_from_probe(kind, probe, in_container),
                    Ok(None) => ResolveOutcome::Skipped(kind),
                    Err(err) => ResolveOutcome::Errored(kind, err),
                }
            })
            .collect()
    }

    fn outcome_from_probe(
        &self,
        source_kind: SourceKind,
        probe: Probe,
        in_container: bool,
    ) -> ResolveOutcome {
        debug_assert_eq!(
            source_kind,
            probe.kind(),
            "source {source_kind:?} returned probe with kind {:?}",
            probe.kind(),
        );
        match self.probe_to_host_id(probe, in_container) {
            Ok(id) => ResolveOutcome::Found(id),
            Err(err) => ResolveOutcome::Errored(source_kind, err),
        }
    }

    fn probe_to_host_id(&self, probe: Probe, in_container: bool) -> Result<HostId, Error> {
        let (kind, raw) = probe.into_parts();
        if raw.trim().is_empty() {
            return Err(malformed_empty(kind));
        }
        self.wrap
            .apply(&raw)
            .map(|uuid| HostId::new(uuid, kind, in_container))
            .ok_or_else(|| malformed_invalid_uuid(kind, &raw))
    }
}

fn malformed_empty(source_kind: SourceKind) -> Error {
    Error::Malformed {
        source_kind,
        reason: EMPTY_RAW_REASON.to_owned(),
    }
}

fn malformed_invalid_uuid(source_kind: SourceKind, raw: &str) -> Error {
    Error::Malformed {
        source_kind,
        reason: format!("value is not a valid UUID: {raw}"),
    }
}

impl Default for Resolver {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl std::fmt::Debug for Resolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolver")
            .field("sources", &self.source_kinds())
            .field("wrap", &self.wrap)
            .finish()
    }
}

#[cfg(target_os = "linux")]
fn detected_container() -> bool {
    sources::linux_in_container()
}

#[cfg(not(target_os = "linux"))]
fn detected_container() -> bool {
    false
}
