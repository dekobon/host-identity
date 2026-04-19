//! NetBSD / OpenBSD: `sysctl kern.hostid`.
//!
//! Authoritative references:
//!
//! - NetBSD [`sysctl(7)` kern.hostid](https://man.netbsd.org/sysctl.7) and
//!   [`sethostid(3)`](https://man.netbsd.org/sethostid.3) — define
//!   `kern.hostid` as a 32-bit integer, with `0` meaning "not set."
//! - OpenBSD [`sysctl(8)`](https://man.openbsd.org/sysctl.8) and
//!   [`gethostid(3)`](https://man.openbsd.org/gethostid.3) — same contract.
//!
//! This source rejects `0` (decimal or any `0x`-prefixed hex form) because
//! both manpages explicitly document it as the unset sentinel.
//!
//! # Identity scope
//!
//! `SysctlKernHostId` is **per-host-OS**: `kern.hostid` is a kernel
//! variable tied to the OS install. NetBSD and OpenBSD don't ship
//! first-class container runtimes, but any tool that virtualises
//! `sysctl` shares the host's view by default, so the source
//! returns the host's identity from inside any such environment.
//! See `docs/algorithm.md` → "Identity scope".
//!
//! # Blocking behaviour
//!
//! Spawns `sysctl` synchronously. Healthy calls return in milliseconds,
//! but `sysctl` can block indefinitely if the kernel is unresponsive.
//! Callers that need a bounded resolver latency should wrap
//! [`crate::Resolver::resolve`] with their own timeout.

use std::process::Command;

use crate::error::Error;
use crate::source::{Probe, Source, SourceKind};
use crate::sources::util::normalize;

/// Reads `kern.hostid` via `sysctl -n kern.hostid`. A hostid of `0` is
/// treated as unset.
#[derive(Debug, Default, Clone)]
pub struct SysctlKernHostId {
    _priv: (),
}

impl SysctlKernHostId {
    /// Construct the source.
    #[must_use]
    pub fn new() -> Self {
        Self { _priv: () }
    }
}

impl Source for SysctlKernHostId {
    fn kind(&self) -> SourceKind {
        SourceKind::BsdKernHostId
    }

    fn probe(&self) -> Result<Option<Probe>, Error> {
        let output = Command::new("/sbin/sysctl")
            .args(["-n", "kern.hostid"])
            .output()
            .map_err(|e| Error::Platform {
                source_kind: SourceKind::BsdKernHostId,
                reason: format!("sysctl: {e}"),
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::debug!(
                "host-identity: sysctl kern.hostid exited with {}: {}",
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
            .map(|v| Probe::new(SourceKind::BsdKernHostId, v)))
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
    fn lowercase_hex_zero_is_zero() {
        assert!(is_zero_hostid("0x0"));
    }

    #[test]
    fn uppercase_hex_zero_padded_is_zero() {
        assert!(is_zero_hostid("0X00000000"));
    }

    #[test]
    fn nonzero_hex_is_not_zero() {
        assert!(!is_zero_hostid("0x1"));
    }

    #[test]
    fn non_numeric_value_is_not_zero() {
        assert!(!is_zero_hostid("deadbeef-not-a-number"));
    }
}
