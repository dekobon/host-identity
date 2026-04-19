//! Portable sources: overrides and custom callbacks.
//!
//! # Identity scope
//!
//! `EnvOverride`, `FileOverride`, and `FnSource` are **caller-scoped**:
//! whoever sets the variable, writes the file, or supplies the closure
//! picks the scope. Setting `HOST_IDENTITY` in a host-level systemd
//! unit gives host scope to every container that inherits the
//! environment; setting the same variable in a Kubernetes pod spec
//! (via `valueFrom.fieldRef.fieldPath: metadata.uid`) gives per-pod
//! scope. The crate cannot tell which you intended — pick
//! intentionally.
//!
//! For per-pod identity sourced from a projected file, prefer the
//! purpose-built [`crate::sources::KubernetesDownwardApi`] over
//! wiring `FileOverride` at a downward-API path: the k8s source
//! labels the probe with `SourceKind::KubernetesDownwardApi` so the
//! resulting [`crate::HostId`] records the correct provenance.
//! Reserve `FileOverride` for genuinely caller-pinned paths (a
//! per-instance volume, an HSM-written file, a provisioning agent's
//! output).
//!
//! See `docs/algorithm.md` → "Identity scope" for the host-vs-
//! container trap these sources are most often used to work around.

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::source::{Probe, Source, SourceKind};
use crate::sources::util::{normalize, read_capped};

/// Read the identifier from an environment variable.
///
/// Skipped (`Ok(None)`) when the variable is unset, empty, or holds only
/// whitespace.
#[derive(Debug, Clone)]
pub struct EnvOverride {
    var: String,
}

impl EnvOverride {
    /// Read from the named environment variable.
    #[must_use]
    pub fn new(var: impl Into<String>) -> Self {
        Self { var: var.into() }
    }
}

impl Source for EnvOverride {
    fn kind(&self) -> SourceKind {
        SourceKind::EnvOverride
    }

    fn probe(&self) -> Result<Option<Probe>, Error> {
        match std::env::var(&self.var) {
            Ok(value) => Ok(normalize(&value).map(|v| Probe::new(SourceKind::EnvOverride, v))),
            Err(_) => Ok(None),
        }
    }
}

/// Read the identifier from a single-line file.
///
/// Skipped when the file does not exist, is empty, or contains only
/// whitespace. Returns an I/O error for permission problems or other
/// unexpected read failures.
#[derive(Debug, Clone)]
pub struct FileOverride {
    path: PathBuf,
}

impl FileOverride {
    /// Read from the given file path.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The configured path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Source for FileOverride {
    fn kind(&self) -> SourceKind {
        SourceKind::FileOverride
    }

    fn probe(&self) -> Result<Option<Probe>, Error> {
        match read_capped(&self.path) {
            Ok(content) => Ok(normalize(&content).map(|v| Probe::new(SourceKind::FileOverride, v))),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(Error::Io {
                source_kind: SourceKind::FileOverride,
                path: self.path.clone(),
                source,
            }),
        }
    }
}

/// A [`Source`] backed by a user-supplied closure.
///
/// Use to plug in arbitrary identity providers (cloud metadata, Kubernetes
/// downward API, hardware security modules, …) without implementing the
/// trait manually.
pub struct FnSource<F> {
    kind: SourceKind,
    f: F,
}

impl<F> FnSource<F>
where
    F: Fn() -> Result<Option<String>, Error> + Send + Sync,
{
    /// Build a source from a closure that returns an optional raw identifier.
    pub fn new(kind: SourceKind, f: F) -> Self {
        Self { kind, f }
    }
}

impl<F> std::fmt::Debug for FnSource<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FnSource")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl<F> Source for FnSource<F>
where
    F: Fn() -> Result<Option<String>, Error> + Send + Sync,
{
    fn kind(&self) -> SourceKind {
        self.kind
    }

    fn probe(&self) -> Result<Option<Probe>, Error> {
        Ok((self.f)()?
            .as_deref()
            .and_then(normalize)
            .map(|v| Probe::new(self.kind, v)))
    }
}
