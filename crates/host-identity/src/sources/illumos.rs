//! illumos / Solaris: `hostid(1)`.
//!
//! Authoritative reference:
//! [illumos `hostid(1)`](https://illumos.org/man/1/hostid) — documents the
//! `/usr/bin/hostid` command as printing the host numeric identifier. On
//! illumos this value is persisted in the kernel and exposed via
//! [`sysinfo(2)`](https://illumos.org/man/2/sysinfo)
//! (`SI_HW_SERIAL` / `SI_HW_PROVIDER`).
//!
//! # Blocking behaviour
//!
//! Spawns `/usr/bin/hostid` synchronously. Normal calls return in
//! milliseconds; under kernel stalls the child can block indefinitely.
//! Wrap [`crate::Resolver::resolve`] with a caller-managed timeout when
//! bounded latency is required.

use std::process::Command;

use crate::error::Error;
use crate::source::{Probe, Source, SourceKind};
use crate::sources::util::normalize;

/// Reads the system hostid via `/usr/bin/hostid`.
#[derive(Debug, Default, Clone)]
pub struct IllumosHostId {
    _priv: (),
}

impl IllumosHostId {
    /// Construct the source.
    #[must_use]
    pub fn new() -> Self {
        Self { _priv: () }
    }
}

impl Source for IllumosHostId {
    fn kind(&self) -> SourceKind {
        SourceKind::IllumosHostId
    }

    fn probe(&self) -> Result<Option<Probe>, Error> {
        let output = Command::new("/usr/bin/hostid")
            .output()
            .map_err(|e| Error::Platform {
                source_kind: SourceKind::IllumosHostId,
                reason: format!("hostid: {e}"),
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::debug!(
                "host-identity: hostid exited with {}: {}",
                output.status,
                stderr.trim()
            );
            return Ok(None);
        }
        let Ok(value) = std::str::from_utf8(&output.stdout) else {
            return Ok(None);
        };
        Ok(normalize(value).map(|v| Probe::new(SourceKind::IllumosHostId, v)))
    }
}
