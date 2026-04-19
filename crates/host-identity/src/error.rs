//! Error type for identity resolution.

use std::io;
use std::path::PathBuf;

use crate::source::SourceKind;

/// Errors returned by [`crate::Resolver::resolve`].
///
/// Every variant except [`Error::NoSource`] carries the [`SourceKind`]
/// that produced it, so logs and error messages unambiguously identify
/// which source failed. [`Error::source_kind`] exposes the field
/// uniformly across variants.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Every configured source was tried and none produced a usable identity.
    #[error("no identity source produced a value (tried: {tried})")]
    NoSource {
        /// Comma-separated list of sources that were attempted.
        tried: String,
    },

    /// A source file was present but contained the systemd `uninitialized`
    /// sentinel. The caller should not treat this as a valid identity — every
    /// host in this state would hash to the same UUID.
    #[error("{source_kind}: {path} contains the `uninitialized` sentinel")]
    Uninitialized {
        /// Which source produced the error.
        source_kind: SourceKind,
        /// Path of the offending file.
        path: PathBuf,
    },

    /// I/O failure while reading a source file. Command-spawn failures are
    /// reported as [`Error::Platform`] instead — this variant's `path`
    /// field is always a real filesystem path.
    #[error("{source_kind}: I/O error reading {}: {source}", path.display())]
    Io {
        /// Which source produced the error.
        source_kind: SourceKind,
        /// Filesystem path that produced the error.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },

    /// A source returned a value that is not a well-formed identifier
    /// (empty after trimming, wrong shape, invalid UTF-8, …).
    #[error("{source_kind}: malformed value: {reason}")]
    Malformed {
        /// Which source produced the error.
        source_kind: SourceKind,
        /// Human-readable reason.
        reason: String,
    },

    /// Platform-specific lookup failed (registry query, syscall, ioreg,
    /// cloud metadata contract violation, …).
    #[error("{source_kind}: {reason}")]
    Platform {
        /// Which source produced the error.
        source_kind: SourceKind,
        /// Human-readable reason.
        reason: String,
    },
}

impl Error {
    /// The source that produced this error, if the variant carries one.
    ///
    /// Returns `None` only for [`Error::NoSource`], which reports that
    /// *every* source was tried and none produced a value — that error
    /// doesn't belong to any single source.
    #[must_use]
    pub fn source_kind(&self) -> Option<SourceKind> {
        match self {
            Self::NoSource { .. } => None,
            Self::Uninitialized { source_kind, .. }
            | Self::Io { source_kind, .. }
            | Self::Malformed { source_kind, .. }
            | Self::Platform { source_kind, .. } => Some(*source_kind),
        }
    }

    /// Whether this error is reasonable for the caller to recover from
    /// at runtime (log a warning, mint a per-run placeholder UUID, etc.)
    /// rather than treat as a fatal configuration problem.
    ///
    /// - [`Error::NoSource`] → `true`. No source produced a value, but
    ///   the crate is behaving correctly; the caller's chain simply
    ///   doesn't match the environment. Apps often handle this by
    ///   falling back to their own ID scheme.
    /// - All other variants → `false`. They indicate a concrete fault
    ///   (sentinel, I/O failure, malformed source value, platform-tool
    ///   failure) that won't fix itself on retry; the caller should
    ///   surface them to the operator.
    ///
    /// This classification is a guideline, not a hard contract. A
    /// particular deployment might reasonably treat an `Io` error on
    /// `/etc/machine-id` as recoverable (keep going with the next
    /// source) — the method exists to give the common case a one-liner,
    /// not to remove the caller's judgement.
    #[must_use]
    pub fn is_recoverable(&self) -> bool {
        matches!(self, Self::NoSource { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_recoverable_only_true_for_no_source() {
        assert!(Error::NoSource { tried: "x".into() }.is_recoverable());
        assert!(
            !Error::Uninitialized {
                source_kind: SourceKind::MachineId,
                path: "/etc/machine-id".into(),
            }
            .is_recoverable()
        );
        assert!(
            !Error::Platform {
                source_kind: SourceKind::IoPlatformUuid,
                reason: "ioreg failed".into(),
            }
            .is_recoverable()
        );
        assert!(
            !Error::Malformed {
                source_kind: SourceKind::Dmi,
                reason: "not a uuid".into(),
            }
            .is_recoverable()
        );
    }

    #[test]
    fn source_kind_round_trips_through_error() {
        let err = Error::Platform {
            source_kind: SourceKind::AwsImds,
            reason: "x".into(),
        };
        assert_eq!(err.source_kind(), Some(SourceKind::AwsImds));
        assert_eq!(
            Error::NoSource {
                tried: "env-override".into()
            }
            .source_kind(),
            None
        );
    }
}
