//! illumos / Solaris: `hostid(1)`.
//!
//! Authoritative reference:
//! [illumos `hostid(1)`](https://illumos.org/man/1/hostid) — documents the
//! `/usr/bin/hostid` command as printing the host numeric identifier. On
//! illumos this value is persisted in the kernel and exposed via
//! [`sysinfo(2)`](https://illumos.org/man/2/sysinfo)
//! (`SI_HW_SERIAL` / `SI_HW_PROVIDER`).
//!
//! # Identity scope
//!
//! `IllumosHostId` is **per-host-OS**: `hostid(1)` reads a kernel
//! value seeded from `/etc/hostid` or zone configuration. Inside a
//! non-global zone the value typically reflects the zone's own
//! configuration rather than the global zone's, but any zone that
//! inherits the global zone's `hostid` (or any container that shares
//! the kernel view) returns the host's identity. See
//! `docs/algorithm.md` → "Identity scope".
//!
//! # Unset sentinel
//!
//! This source rejects `0` (decimal or any `0x`-prefixed hex form,
//! including the canonical `00000000` that `hostid(1)` prints on a
//! factory-fresh or unset host). illumos `sysinfo(2)` documents
//! `SI_HW_SERIAL` as `"0"` when unset, so a zero reading is the
//! platform's "not configured" signal — not a valid identity.
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
        Ok(normalize(value)
            .filter(|v| !is_zero_hostid(v))
            .map(|v| Probe::new(SourceKind::IllumosHostId, v)))
    }
}

fn is_zero_hostid(v: &str) -> bool {
    let (digits, radix) = v
        .strip_prefix("0x")
        .or_else(|| v.strip_prefix("0X"))
        .map_or((v, 10), |d| (d, 16));
    u64::from_str_radix(digits, radix).is_ok_and(|n| n == 0)
}

#[cfg(test)]
mod tests {
    use super::is_zero_hostid;

    #[test]
    fn decimal_zero_is_zero() {
        assert!(is_zero_hostid("0"));
    }

    #[test]
    fn hex_zero_padded_is_zero() {
        assert!(is_zero_hostid("00000000"));
    }

    #[test]
    fn lowercase_hex_prefix_zero_is_zero() {
        assert!(is_zero_hostid("0x0"));
    }

    #[test]
    fn uppercase_hex_prefix_zero_is_zero() {
        assert!(is_zero_hostid("0X00000000"));
    }

    #[test]
    fn nonzero_hex_is_not_zero() {
        assert!(!is_zero_hostid("4f988f8f"));
    }

    #[test]
    fn non_numeric_value_is_not_zero() {
        assert!(!is_zero_hostid("deadbeef-not-a-number"));
    }
}
