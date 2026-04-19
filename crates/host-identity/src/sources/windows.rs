//! Windows: `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid`.
//!
//! The `MachineGuid` value under the Cryptography subkey is written once by
//! Windows Setup and persists across reboots, user-profile changes, and CNG
//! re-keying. Microsoft does not publish a dedicated spec page for this
//! value, but it is documented as a stable per-install identifier in the
//! [CNG Registry Keys reference](https://learn.microsoft.com/en-us/windows/win32/seccng/cng-registry-keys).
//! The standard access contract is the
//! [Windows Registry API](https://learn.microsoft.com/en-us/windows/win32/sysinfo/registry),
//! which this source reaches through the `windows-registry` crate.
//!
//! # Identity scope
//!
//! `MachineGuid` is **per-host-OS** on the bare host: written once by
//! Windows Setup and tied to the install.
//!
//! In Windows containers the picture is different from Linux.
//! Microsoft documents the Windows registry as one of the namespaces
//! *virtualized per-container* in both process-isolation and
//! Hyper-V–isolation modes — see
//! [Isolation modes (Microsoft Learn)](https://learn.microsoft.com/en-us/virtualization/windowscontainers/manage-containers/hyperv-container)
//! ("Windows containers virtualize access to various operating
//! system namespaces … file system, registry, network ports,
//! process and thread ID space, Object Manager namespace"). A
//! process-isolated container therefore reads the `MachineGuid` that
//! was baked into the container *image* at build time, not the
//! host's — which produces a different collision mode from Linux:
//! every container started from the same base image shares one
//! `MachineGuid`, independent of which host runs them. Hyper-V–
//! isolated containers likewise see their own registry inside their
//! utility VM.
//!
//! The defence is the same in either case: a chain that wants
//! per-container identity must place a container-scope source above
//! `WindowsMachineGuid`. See `docs/algorithm.md` → "Identity scope".

use crate::error::Error;
use crate::source::{Probe, Source, SourceKind};
use crate::sources::util::normalize;

const KEY_PATH: &str = r"SOFTWARE\Microsoft\Cryptography";
const FULL_KEY: &str = r"HKLM\SOFTWARE\Microsoft\Cryptography";
const VALUE_NAME: &str = "MachineGuid";

/// Reads `MachineGuid` from the Cryptography registry key. Written once at
/// install time and persists across reboots and user-account changes.
#[derive(Debug, Default, Clone)]
pub struct WindowsMachineGuid {
    _priv: (),
}

impl WindowsMachineGuid {
    /// Construct the source.
    #[must_use]
    pub fn new() -> Self {
        Self { _priv: () }
    }
}

impl Source for WindowsMachineGuid {
    fn kind(&self) -> SourceKind {
        SourceKind::WindowsMachineGuid
    }

    fn probe(&self) -> Result<Option<Probe>, Error> {
        let key = match windows_registry::LOCAL_MACHINE.open(KEY_PATH) {
            Ok(key) => key,
            Err(err) if is_benign_registry_error(&err) => {
                log::debug!("host-identity: windows-machine-guid: open {FULL_KEY}: {err}");
                return Ok(None);
            }
            Err(err) => {
                return Err(Error::Platform {
                    source_kind: SourceKind::WindowsMachineGuid,
                    reason: format!("open {FULL_KEY}: {err}"),
                });
            }
        };
        let value = match key.get_string(VALUE_NAME) {
            Ok(v) => v,
            Err(err) if is_benign_registry_error(&err) => {
                log::debug!("host-identity: windows-machine-guid: read {VALUE_NAME}: {err}");
                return Ok(None);
            }
            Err(err) => {
                return Err(Error::Platform {
                    source_kind: SourceKind::WindowsMachineGuid,
                    reason: format!("read {VALUE_NAME}: {err}"),
                });
            }
        };
        Ok(probe_from_registry_value(&value))
    }
}

/// `ERROR_FILE_NOT_FOUND` (2) and `ERROR_ACCESS_DENIED` (5) are Windows
/// errors that other platforms map to `Ok(None)` (missing file /
/// permission denied). Treat them the same here so the resolver can
/// fall through to the next source on hardened or minimal installs.
fn is_benign_registry_error(err: &windows_result::Error) -> bool {
    let code = err.code().0 as u32 & 0xFFFF;
    code == 2 || code == 5
}

/// Map a raw registry value string to an optional probe.
///
/// Windows differs from `MachineIdFile` / `DmiProductUuid` here — when the
/// sentinel `uninitialized` is read, we return `Ok(None)` rather than an
/// error. The registry value is written at install time, not by an early-
/// boot daemon, so a sentinel means "something wrote a placeholder" and we
/// simply fall through to the next source.
fn probe_from_registry_value(value: &str) -> Option<Probe> {
    normalize(value).map(|v| Probe::new(SourceKind::WindowsMachineGuid, v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentinel_registry_value_yields_none() {
        // Pins the Windows-specific design decision: the `uninitialized`
        // sentinel short-circuits to `Ok(None)` so the resolver falls
        // through, rather than erroring as the Linux sources do.
        assert!(probe_from_registry_value("uninitialized").is_none());
        assert!(probe_from_registry_value("UNINITIALIZED\r\n").is_none());
    }

    #[test]
    fn empty_registry_value_yields_none() {
        assert!(probe_from_registry_value("").is_none());
        assert!(probe_from_registry_value("   ").is_none());
    }

    #[test]
    fn usable_registry_value_yields_probe() {
        let probe = probe_from_registry_value("12345678-1234-1234-1234-123456789ABC")
            .expect("usable value should yield a probe");
        assert_eq!(probe.kind(), SourceKind::WindowsMachineGuid);
        assert_eq!(probe.value(), "12345678-1234-1234-1234-123456789ABC");
    }
}
