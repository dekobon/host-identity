//! FreeBSD: `/etc/hostid`, then `kenv smbios.system.uuid`.
//!
//! Authoritative references:
//!
//! - [`hostid(1)`](https://man.freebsd.org/cgi/man.cgi?query=hostid&sektion=1)
//!   and [`gethostid(3)`](https://man.freebsd.org/cgi/man.cgi?query=gethostid&sektion=3)
//!   — document `/etc/hostid` as the persistent host identifier file. The
//!   rc.d system (`rc.d/hostid`) generates a UUID on first boot if the file
//!   is absent.
//! - [`kenv(1)`](https://man.freebsd.org/cgi/man.cgi?query=kenv&sektion=1) —
//!   reads kernel environment variables. The `smbios.system.uuid` variable
//!   is populated from the SMBIOS type 1 system UUID
//!   ([DMTF DSP0134](https://www.dmtf.org/dsp/DSP0134)).
//!
//! # Identity scope
//!
//! These sources split across two scopes:
//!
//! - `FreeBsdHostIdFile` is **per-host-OS**: written once by
//!   `rc.d/hostid` on first boot and tied to the install.
//! - `KenvSmbios` is **per-instance**: the SMBIOS system UUID is
//!   assigned by the hypervisor (on VMs) or the OEM (on bare metal).
//!
//! FreeBSD jails share `/etc/hostid` and the host's `kenv` view by
//! default, so either source read from inside a jail returns the
//! host's identity — the same host-vs-container trap as Linux. See
//! `docs/algorithm.md` → "Identity scope".
//!
//! # Blocking behaviour
//!
//! [`KenvSmbios`] spawns `kenv` synchronously. Normal calls return in
//! milliseconds, but the child can block indefinitely under kernel
//! stalls — wrap [`crate::Resolver::resolve`] with a caller timeout if
//! a bounded latency matters.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::Error;
use crate::source::{Probe, Source, SourceKind};
use crate::sources::util::{normalize, read_capped};

const HOSTID_PATH: &str = "/etc/hostid";

/// Reads `/etc/hostid`.
#[derive(Debug, Clone)]
pub struct FreeBsdHostIdFile {
    path: PathBuf,
}

impl FreeBsdHostIdFile {
    /// Read from the standard path.
    #[must_use]
    pub fn new() -> Self {
        Self {
            path: PathBuf::from(HOSTID_PATH),
        }
    }

    /// Read from a caller-supplied path.
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The configured path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Default for FreeBsdHostIdFile {
    fn default() -> Self {
        Self::new()
    }
}

impl Source for FreeBsdHostIdFile {
    fn kind(&self) -> SourceKind {
        SourceKind::FreeBsdHostId
    }

    fn probe(&self) -> Result<Option<Probe>, Error> {
        match read_capped(&self.path) {
            Ok(content) => {
                let Some(value) = normalize(&content) else {
                    return Ok(None);
                };
                // Reject binary / non-printable payloads. FreeBSD's
                // rc.d/hostid writes a UUID string, but `sethostid(2)`
                // can populate /etc/hostid with raw bytes on some
                // configurations — those must not flow through as an
                // identifier.
                if !value.bytes().all(is_printable_ascii) {
                    return Err(Error::Malformed {
                        source_kind: SourceKind::FreeBsdHostId,
                        reason: "hostid file contains non-printable bytes".to_owned(),
                    });
                }
                Ok(Some(Probe::new(SourceKind::FreeBsdHostId, value)))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(Error::Io {
                source_kind: SourceKind::FreeBsdHostId,
                path: self.path.clone(),
                source,
            }),
        }
    }
}

fn is_printable_ascii(b: u8) -> bool {
    matches!(b, 0x20..=0x7e)
}

/// Reads the SMBIOS system UUID via `kenv -q smbios.system.uuid`.
#[derive(Debug, Default, Clone)]
pub struct KenvSmbios {
    _priv: (),
}

impl KenvSmbios {
    /// Construct the source.
    #[must_use]
    pub fn new() -> Self {
        Self { _priv: () }
    }
}

impl Source for KenvSmbios {
    fn kind(&self) -> SourceKind {
        SourceKind::KenvSmbios
    }

    fn probe(&self) -> Result<Option<Probe>, Error> {
        let output = Command::new("/bin/kenv")
            .args(["-q", "smbios.system.uuid"])
            .output()
            .map_err(|e| Error::Platform {
                source_kind: SourceKind::KenvSmbios,
                reason: format!("kenv: {e}"),
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::debug!(
                "host-identity: kenv exited with {}: {}",
                output.status,
                stderr.trim()
            );
            return Ok(None);
        }
        let Ok(value) = std::str::from_utf8(&output.stdout) else {
            return Ok(None);
        };
        Ok(normalize(value).map(|v| Probe::new(SourceKind::KenvSmbios, v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn printable_ascii_boundary() {
        assert!(!is_printable_ascii(0x1f));
        assert!(is_printable_ascii(0x20));
        assert!(is_printable_ascii(0x7e));
        assert!(!is_printable_ascii(0x7f));
    }

    #[test]
    fn hostid_file_reads_uuid() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, "12345678-1234-1234-1234-123456789abc").unwrap();
        let probe = FreeBsdHostIdFile::at(f.path()).probe().unwrap().unwrap();
        assert_eq!(probe.kind(), SourceKind::FreeBsdHostId);
        assert_eq!(probe.value(), "12345678-1234-1234-1234-123456789abc");
    }

    #[test]
    fn hostid_file_missing_is_none() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(
            FreeBsdHostIdFile::at(dir.path().join("hostid"))
                .probe()
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn hostid_file_empty_is_none() {
        let f = tempfile::NamedTempFile::new().unwrap();
        assert!(FreeBsdHostIdFile::at(f.path()).probe().unwrap().is_none());
    }

    #[test]
    fn hostid_file_non_printable_bytes_errors() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(&[0x00, 0x01, 0x02, 0x03]).unwrap();
        match FreeBsdHostIdFile::at(f.path()).probe() {
            Err(Error::Malformed {
                source_kind,
                reason,
            }) => {
                assert_eq!(source_kind, SourceKind::FreeBsdHostId);
                assert!(reason.contains("non-printable"));
            }
            other => panic!("expected Error::Malformed, got {other:?}"),
        }
    }

    #[test]
    fn hostid_file_reports_io_error_for_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        match FreeBsdHostIdFile::at(dir.path()).probe() {
            Err(Error::Io { path, .. }) => assert_eq!(path, dir.path()),
            other => panic!("expected Error::Io, got {other:?}"),
        }
    }
}
