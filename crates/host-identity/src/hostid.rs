//! The resolved [`HostId`] value.

use std::fmt;

use uuid::Uuid;

use crate::source::SourceKind;

/// A stable host identifier.
///
/// A `HostId` is always a UUID, but wraps additional provenance: which source
/// produced the raw value and whether the host was running inside a container
/// at resolution time. The wire representation (via [`HostId::as_uuid`] or
/// [`fmt::Display`]) is the hyphenated UUID string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HostId {
    uuid: Uuid,
    source: SourceKind,
    in_container: bool,
}

impl HostId {
    pub(crate) fn new(uuid: Uuid, source: SourceKind, in_container: bool) -> Self {
        Self {
            uuid,
            source,
            in_container,
        }
    }

    /// The identifier as a [`Uuid`].
    #[must_use]
    pub fn as_uuid(&self) -> Uuid {
        self.uuid
    }

    /// Which source the raw value was read from.
    #[must_use]
    pub fn source(&self) -> SourceKind {
        self.source
    }

    /// Whether the resolver detected a container runtime at resolution time.
    ///
    /// When `true`, [`HostId::source`] reflects the container branch rather
    /// than a host-level source.
    ///
    /// On non-Linux targets this is always `false` — container-runtime
    /// detection is implemented via `/.dockerenv` and `/proc/1/cgroup`,
    /// both Linux-only. A macOS or Windows host running Docker Desktop
    /// will still report `false` because the host process itself is not
    /// inside the container namespace.
    #[must_use]
    pub fn in_container(&self) -> bool {
        self.in_container
    }

    /// Log-friendly summary combining source kind and UUID.
    ///
    /// Returns a value that implements [`fmt::Display`] as
    /// `"{source_kind}:{uuid}"`, e.g. `"aws-imds:i-0abc…"` becomes
    /// `"aws-imds:12345678-1234-…"` after wrapping. Keeps `HostId`'s own
    /// `Display` impl wire-clean (just the UUID) while giving operators
    /// the provenance tag they usually want in logs.
    ///
    /// ```
    /// # use host_identity::{HostId, Resolver, sources::EnvOverride};
    /// # // SAFETY: test-only env manipulation.
    /// # unsafe { std::env::set_var("HOST_IDENTITY_TEST_SUMMARY", "x") };
    /// # let id = Resolver::new()
    /// #     .push(EnvOverride::new("HOST_IDENTITY_TEST_SUMMARY"))
    /// #     .resolve().unwrap();
    /// # unsafe { std::env::remove_var("HOST_IDENTITY_TEST_SUMMARY") };
    /// let s = id.summary().to_string();
    /// assert!(s.starts_with("env-override:"));
    /// ```
    #[must_use]
    pub fn summary(&self) -> HostIdSummary<'_> {
        HostIdSummary(self)
    }
}

/// `Display` wrapper returned by [`HostId::summary`].
///
/// Formats as `"{source_kind}:{uuid}"`. Not constructible directly;
/// callers get an instance from `HostId::summary`.
#[derive(Debug)]
pub struct HostIdSummary<'a>(&'a HostId);

impl fmt::Display for HostIdSummary<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.0.source, self.0.uuid)
    }
}

impl fmt::Display for HostId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.uuid.fmt(f)
    }
}

/// One source's outcome in a full-chain walk.
///
/// Returned by [`crate::Resolver::resolve_all`] (and the free
/// [`crate::resolve_all`] / [`crate::resolve_all_with_transport`] wrappers)
/// for every source in the chain, in chain order. Unlike [`crate::Resolver::resolve`],
/// a full walk does **not** short-circuit on the first success or the first
/// error — every source is consulted.
///
/// Use this when you want to audit what each source would produce (diagnostics,
/// operator tooling, test harnesses). For normal resolution use
/// [`crate::resolve`] — it stops at the first usable source.
#[derive(Debug)]
pub enum ResolveOutcome {
    /// The source produced a usable identifier.
    Found(HostId),
    /// The source had nothing to offer (file absent, endpoint unreachable,
    /// feature disabled, wrong platform).
    Skipped(SourceKind),
    /// The source produced a hard error. In a short-circuiting `resolve()`
    /// this would have aborted the chain; in `resolve_all` the error is
    /// captured here and the walk continues.
    ///
    /// The outer [`SourceKind`] and the inner [`crate::Error::source_kind`]
    /// are guaranteed to be equal. The field on this variant is the
    /// authoritative provenance for callers matching outcomes —
    /// introspecting the inner `Error` for its kind is equivalent but
    /// noisier.
    Errored(SourceKind, crate::Error),
}

impl ResolveOutcome {
    /// Which source produced this outcome.
    #[must_use]
    pub fn source(&self) -> SourceKind {
        match self {
            Self::Found(id) => id.source(),
            Self::Skipped(kind) | Self::Errored(kind, _) => *kind,
        }
    }

    /// The `HostId` if the source produced one, else `None`.
    #[must_use]
    pub fn host_id(&self) -> Option<&HostId> {
        match self {
            Self::Found(id) => Some(id),
            Self::Skipped(_) | Self::Errored(_, _) => None,
        }
    }
}
