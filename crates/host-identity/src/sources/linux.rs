//! Linux identity sources: `/etc/machine-id`, D-Bus machine-id, SMBIOS/DMI.
//!
//! # Identity scope
//!
//! These sources live at two distinct scopes:
//!
//! - `MachineIdFile` and `DbusMachineIdFile` are **per-host-OS**:
//!   written once when the OS is provisioned and tied to the install.
//! - `DmiProductUuid` is **per-instance**: the SMBIOS system UUID is
//!   assigned by the hypervisor (on VMs) or the OEM (on bare metal)
//!   and identifies the hardware/VM, not the OS install.
//!
//! In container deployments the distinction collapses: none of these
//! namespaces are container-isolated, so a process inside a container
//! reads the same value every sibling container on that host reads.
//! `/sys/devices/virtual/dmi/id/product_uuid` isn't namespaced at all
//! — the container sees the underlying VM's SMBIOS UUID directly.
//! Red Hat container images go further and bind-mount the host's
//! `/etc/machine-id` into the container, so even the "file" path
//! leaks host identity into the container. See Docker community
//! discussion of [host `machine-id` visibility in containers](https://forums.docker.com/t/host-machine-id-visible-from-containers/100533)
//! and the sysbox issue [open sys/devices/virtual/dmi/id/product_uuid](https://github.com/nestybox/sysbox/issues/405)
//! for the non-namespaced sysfs path.
//!
//! `ContainerId` (and, in pods, `KubernetesPodUid`) must sit above
//! these sources in any chain that wants per-container identity; the
//! default chains do this for you. See `docs/algorithm.md` →
//! "Identity scope" for the full discussion.
//!
//! Authoritative references:
//!
//! - [`machine-id(5)`](https://www.freedesktop.org/software/systemd/man/latest/machine-id.html)
//!   — systemd-managed per-host identifier, initialised once on first boot.
//!   The `uninitialized` sentinel is specified there as the marker for the
//!   early-boot window before the ID has been written.
//! - [D-Bus specification, UUIDs](https://dbus.freedesktop.org/doc/dbus-specification.html#uuids)
//!   — defines `/var/lib/dbus/machine-id` as the interoperable machine UUID.
//!   On systemd systems this is a symlink to `/etc/machine-id`.
//! - [`sysfs-class-dmi(5)` / kernel sysfs-firmware-dmi-tables](https://www.kernel.org/doc/Documentation/ABI/testing/sysfs-firmware-dmi-tables)
//!   and the SMBIOS specification from the
//!   [DMTF](https://www.dmtf.org/dsp/DSP0134) — `/sys/class/dmi/id/product_uuid`
//!   exposes the SMBIOS system UUID (type 1 "UUID" field). Readable by root
//!   only on most distributions; this crate swallows `PermissionDenied` to
//!   let unprivileged callers fall through to other sources.

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::source::{Probe, Source, SourceKind};
use crate::sources::util::{NormalizeOutcome, classify, read_capped};

macro_rules! file_source {
    ($name:ident, $kind:expr, $default:expr, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone)]
        pub struct $name {
            path: PathBuf,
        }

        impl $name {
            #[doc = concat!("Read from the standard path (`", $default, "`).")]
            #[must_use]
            pub fn new() -> Self {
                Self {
                    path: PathBuf::from($default),
                }
            }

            /// Read from a caller-supplied path. Useful for tests and unusual
            /// image layouts.
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

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl Source for $name {
            fn kind(&self) -> SourceKind {
                $kind
            }
            fn probe(&self) -> Result<Option<Probe>, Error> {
                read_id_file($kind, &self.path)
            }
        }
    };
}

file_source!(
    MachineIdFile,
    SourceKind::MachineId,
    "/etc/machine-id",
    "`/etc/machine-id` — the systemd-managed primary host identifier on modern Linux."
);

file_source!(
    DbusMachineIdFile,
    SourceKind::DbusMachineId,
    "/var/lib/dbus/machine-id",
    "`/var/lib/dbus/machine-id` — D-Bus machine ID. Often a symlink to `/etc/machine-id` \
     but present on its own on some minimal images."
);

file_source!(
    DmiProductUuid,
    SourceKind::Dmi,
    "/sys/class/dmi/id/product_uuid",
    "`/sys/class/dmi/id/product_uuid` — SMBIOS system UUID. Distinct per physical or \
     virtual hardware, so it distinguishes cloned VMs that share a machine-id, but requires \
     root on most distributions."
);

fn read_id_file(kind: SourceKind, path: &Path) -> Result<Option<Probe>, Error> {
    match read_capped(path) {
        Ok(content) => match classify(&content) {
            NormalizeOutcome::Usable(value) => Ok(Some(Probe::new(kind, value))),
            NormalizeOutcome::Sentinel => Err(Error::Uninitialized {
                source_kind: kind,
                path: PathBuf::from(path),
            }),
            NormalizeOutcome::Empty => Ok(None),
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            log::debug!(
                "host-identity: permission denied reading {}",
                path.display()
            );
            Ok(None)
        }
        Err(source) => Err(Error::Io {
            source_kind: kind,
            path: PathBuf::from(path),
            source,
        }),
    }
}

/// Heuristic container-runtime detection.
///
/// Mirrors the checks agent-go uses: `/.dockerenv` existence, runtime markers
/// in `/proc/1/cgroup`. Used by [`crate::HostId::in_container`] for
/// provenance; does not affect which source is chosen (that is the resolver's
/// job — add or remove [`crate::sources::ContainerId`] to change behaviour).
#[must_use]
pub(crate) fn in_container() -> bool {
    const MARKERS: &[&str] = &["docker", "kubepods", "containerd", "podman", "lxc", "crio"];
    Path::new("/.dockerenv").exists()
        || std::fs::read_to_string("/proc/1/cgroup").is_ok_and(|cgroup| {
            cgroup
                .split(['/', ':', '-', '.', '_', '\n'])
                .any(|seg| MARKERS.contains(&seg))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    #[test]
    fn machine_id_file_rejects_uninitialized_sentinel() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "uninitialized").unwrap();
        let err = read_id_file(SourceKind::MachineId, f.path()).expect_err("sentinel must error");
        match err {
            Error::Uninitialized { path, source_kind } => {
                assert_eq!(path, f.path());
                assert_eq!(source_kind, SourceKind::MachineId);
            }
            other => panic!("expected Uninitialized, got {other:?}"),
        }
    }

    #[test]
    fn machine_id_file_accepts_normal_value() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "abc123").unwrap();
        let probe = read_id_file(SourceKind::MachineId, f.path())
            .unwrap()
            .unwrap();
        assert_eq!(probe.value(), "abc123");
    }

    #[test]
    fn machine_id_file_missing_is_none() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("definitely-not-there");
        let probe = read_id_file(SourceKind::MachineId, &missing).unwrap();
        assert!(probe.is_none());
    }

    #[test]
    fn machine_id_file_empty_is_none() {
        let f = NamedTempFile::new().unwrap();
        let probe = read_id_file(SourceKind::MachineId, f.path()).unwrap();
        assert!(probe.is_none());
    }

    #[test]
    fn machine_id_file_whitespace_only_is_none() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "   \n\t ").unwrap();
        let probe = read_id_file(SourceKind::MachineId, f.path()).unwrap();
        assert!(probe.is_none());
    }

    #[test]
    fn machine_id_file_reports_io_error_for_directory() {
        // read_to_string on a directory hits the generic IO arm and must
        // surface as Error::Io carrying the path.
        let dir = TempDir::new().unwrap();
        let err = read_id_file(SourceKind::MachineId, dir.path())
            .expect_err("reading a directory must error");
        match err {
            Error::Io { path, .. } => assert_eq!(path, dir.path()),
            other => panic!("expected Io, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn machine_id_file_permission_denied_is_none() {
        use std::os::unix::fs::PermissionsExt;
        use std::path::{Path, PathBuf};

        /// Restores the file's readable permissions on drop so a panic
        /// mid-test can't leave the tempfile unreadable (which would
        /// break tempfile cleanup).
        struct PermGuard(PathBuf);
        impl Drop for PermGuard {
            fn drop(&mut self) {
                let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o600));
            }
        }

        // Skip when running as root — chmod 0o000 doesn't deny root.
        if nix_is_root() {
            return;
        }

        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "abc123").unwrap();
        let path: &Path = f.path();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let _guard = PermGuard(path.to_path_buf());

        let probe = read_id_file(SourceKind::MachineId, path)
            .expect("permission denied should be swallowed to Ok(None)");
        assert!(probe.is_none());
    }

    #[cfg(unix)]
    fn nix_is_root() -> bool {
        // Avoid pulling in a new dep — read `id -u` via libc would also work,
        // but checking the effective UID via /proc/self/status is trivial.
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("Uid:"))
                    .and_then(|l| l.split_whitespace().next().map(str::to_owned))
            })
            .is_some_and(|uid| uid == "0")
    }
}
