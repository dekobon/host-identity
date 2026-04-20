//! Linux identity sources: `/etc/machine-id`, D-Bus machine-id, SMBIOS/DMI,
//! and the opt-in glibc `/etc/hostid` binary file.
//!
//! # Identity scope
//!
//! These sources live at two distinct scopes:
//!
//! - `MachineIdFile`, `DbusMachineIdFile`, and `LinuxHostIdFile` are
//!   **per-host-OS**: written once when the OS is provisioned (or, for
//!   `/etc/hostid`, by `sethostid(2)` / `zgenhostid` / the image build)
//!   and tied to the install.
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
//! - GNU coreutils [`hostid(1)`](https://www.gnu.org/software/coreutils/hostid),
//!   Linux [`gethostid(3)`](https://man7.org/linux/man-pages/man3/gethostid.3.html),
//!   and [`sethostid(2)`](https://man7.org/linux/man-pages/man2/sethostid.2.html)
//!   — document `/etc/hostid` as four raw bytes in native byte order.
//!   `LinuxHostIdFile` reads the file directly; see its rustdoc for why the
//!   `gethostid(3)` fallback (fabricated from `gethostname()`) is not used.

use std::io::Read;
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
                read_machine_id_file($kind, &self.path)
            }
        }
    };
}

file_source!(
    MachineIdFile,
    SourceKind::MachineId,
    "/etc/machine-id",
    "`/etc/machine-id` — the systemd-managed primary host identifier on modern Linux.\n\n\
     # Known-duplicate filtering\n\n\
     A non-trivial fraction of Linux installs ship or end up with machine-id\n\
     values that are identical across many machines (Whonix's deliberate\n\
     anti-fingerprinting constant; official container images that bake a\n\
     single hex value into the filesystem layer; synthetic all-same-nibble\n\
     values from broken image builds). Returning one of those would produce\n\
     a silently non-unique identity shared by every host that inherits it,\n\
     so this source additionally rejects, by returning `Ok(None)` with a\n\
     `log::debug!` entry:\n\n\
     - A curated list of public, citable shared values (`MACHINE_ID_DENYLIST`).\n\
     - Any 32-hex-digit value whose nibbles are all the same character\n\
       (`00…0`, `11…1`, `aa…a`, etc.). The systemd spec forbids all-zero\n\
       machine-ids outright; the rest are only ever seen on synthetic or\n\
       corrupt images.\n\n\
     Anything not matching the filter passes through unchanged — the intent\n\
     is to reject *known* garbage, not to gate on machine-id shape. A false\n\
     positive here drops a legitimate host from identity resolution, so a\n\
     missing entry is strictly preferable to an over-broad rule."
);

file_source!(
    DbusMachineIdFile,
    SourceKind::DbusMachineId,
    "/var/lib/dbus/machine-id",
    "`/var/lib/dbus/machine-id` — D-Bus machine ID. Often a symlink to `/etc/machine-id` \
     but present on its own on some minimal images. Shares the same \
     known-duplicate filter as [`MachineIdFile`]."
);

/// `/sys/class/dmi/id/product_uuid` — SMBIOS system UUID. Distinct per
/// physical or virtual hardware, so it distinguishes cloned VMs that share
/// a machine-id, but requires root on most distributions.
///
/// # Vendor-placeholder filtering
///
/// SMBIOS commonly ships vendor-default values that are stable *per model*,
/// not per machine. Returning one of those would produce a silently
/// non-unique identity shared by every box with the same mainboard. This
/// source additionally rejects, by returning `Ok(None)` with a
/// `log::debug!` entry:
///
/// - `00000000-0000-0000-0000-000000000000` (all-zero)
/// - `ffffffff-ffff-ffff-ffff-ffffffffffff` (all-F, case-insensitive)
/// - Any UUID whose 32 hex nibbles are all the same character
///   (`11111111-…`, `aaaaaaaa-…`, etc.)
/// - A conservative curated list of well-known vendor placeholders
///   (e.g. `03000200-0400-0500-0006-000700080009`), sourced from
///   [fwupd](https://github.com/fwupd/fwupd) and `dmidecode`.
///
/// Anything not matching the filter passes through unchanged — the intent
/// is to reject *known* garbage, not to gate on UUID shape.
#[derive(Debug, Clone)]
pub struct DmiProductUuid {
    path: PathBuf,
}

impl DmiProductUuid {
    /// Read from the standard path (`/sys/class/dmi/id/product_uuid`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            path: PathBuf::from("/sys/class/dmi/id/product_uuid"),
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

impl Default for DmiProductUuid {
    fn default() -> Self {
        Self::new()
    }
}

impl Source for DmiProductUuid {
    fn kind(&self) -> SourceKind {
        SourceKind::Dmi
    }
    fn probe(&self) -> Result<Option<Probe>, Error> {
        read_dmi_file(&self.path)
    }
}

/// Known-duplicate `/etc/machine-id` values, stored lowercase. Each entry is
/// a public, citable shared value that every host reading the same image or
/// install will produce identically — hashing it would silently collide
/// `HostId`s across unrelated machines. Kept deliberately conservative: a
/// missing entry means the value passes through, which is the less-bad
/// failure mode versus a false positive dropping a legitimate host.
///
/// Container-image entries can rotate when upstream rebuilds the image;
/// each entry carries the source image and observation date so a future
/// maintainer can re-scan and prune obsolete values.
const MACHINE_ID_DENYLIST: &[&str] = &[
    // Whonix / Kicksecure deliberate anti-fingerprinting constant, shipped
    // identically on every install.
    // https://www.whonix.org/wiki/Protocol-Leak-Protection_and_Fingerprinting-Protection
    "b08dfa6083e7567a1921a715000001fb",
    // docker.io/library/oraclelinux:9 — observed 2026-04-19.
    "d495c4b7bb8244639186ef65305fd685",
    // docker.io/library/oraclelinux:8 — observed 2026-04-19.
    "e28a15f597cd4693bb61f1f3e8447cbd",
    // jrei/systemd-debian:latest — popular systemd-enabled base for
    // Ansible/Molecule testing. Observed 2026-04-19.
    "4c010dc413ad444698de6ee4677331b9",
    // jrei/systemd-ubuntu:latest — observed 2026-04-19.
    "a7570853ab864bbbbfc8c54b14eeaf8f",
    // geerlingguy/docker-ubuntu2204-ansible:latest — observed 2026-04-19.
    "5b4bb40898b2416087b6224f176978fb",
    // geerlingguy/docker-debian12-ansible:latest — observed 2026-04-19.
    "3948e4ca87b64871b31c9a49920b9834",
    // geerlingguy/docker-rockylinux9-ansible:latest — observed 2026-04-19.
    "835aa90928e143e3ae09efcd0c5cb118",
];

/// Return `true` if `value` is a known-duplicate machine-id that should be
/// rejected rather than used as an identity.
fn is_machine_id_garbage(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    MACHINE_ID_DENYLIST.contains(&lower.as_str()) || is_all_same_nibble_hex32(&lower)
}

/// Return `true` if `value` is exactly 32 hex digits and every digit is
/// the same character. Covers the systemd-forbidden all-zero case and the
/// synthetic `"11"*32`, `"aa"*32`, etc. values seen on broken images.
///
/// Deliberately **not** unified with [`is_all_same_nibble_uuid`]: that
/// predicate accepts hyphenated 8-4-4-4-12 UUIDs (SMBIOS/DMI format);
/// this one rejects hyphens because machine-id is specified as exactly
/// 32 hex digits with no separators.
fn is_all_same_nibble_hex32(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 32 && bytes[0].is_ascii_hexdigit() && bytes.iter().all(|b| *b == bytes[0])
}

fn read_machine_id_file(kind: SourceKind, path: &Path) -> Result<Option<Probe>, Error> {
    match read_id_file(kind, path)? {
        Some(probe) if is_machine_id_garbage(probe.value()) => {
            log::debug!(
                "host-identity: {kind:?} value {} matches a known-duplicate machine-id; \
                 falling through",
                probe.value()
            );
            Ok(None)
        }
        other => Ok(other),
    }
}

/// Well-known vendor-placeholder UUIDs, stored lowercase. Sourced from
/// fwupd's UEFI plugin quirks list and `dmidecode` field notes. Kept
/// deliberately conservative — a missing entry means the value passes
/// through, which is the less-bad failure mode.
const DMI_PLACEHOLDER_UUIDS: &[&str] = &[
    // Supermicro / AMI golden default seen on a wide range of boards.
    "03000200-0400-0500-0006-000700080009",
];

/// Return `true` if `value` looks like SMBIOS vendor-default garbage that
/// should be rejected rather than used as an identity.
fn is_dmi_garbage(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if DMI_PLACEHOLDER_UUIDS.iter().any(|p| *p == lower) {
        return true;
    }
    is_all_same_nibble_uuid(&lower)
}

/// Return `true` if the input is a canonical 8-4-4-4-12 hyphenated UUID
/// whose 32 hex nibbles are all the same character. Subsumes the
/// all-zero and all-F cases and rejects `11111111-…`, `aaaaaaaa-…`, etc.
///
/// The 32-hex-digit gate keeps short non-UUID values like `"abc"` from
/// false-positively hitting this rule.
///
/// Deliberately **not** unified with [`is_all_same_nibble_hex32`]: that
/// predicate requires exactly 32 hex digits with no hyphens (machine-id
/// format); this one accepts hyphenated 8-4-4-4-12 UUIDs (SMBIOS/DMI
/// format).
fn is_all_same_nibble_uuid(value: &str) -> bool {
    let mut chars = value.chars().filter(|c| *c != '-');
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_hexdigit() {
        return false;
    }
    let mut count = 1usize;
    for c in chars {
        if c != first {
            return false;
        }
        count += 1;
    }
    count == 32
}

fn read_dmi_file(path: &Path) -> Result<Option<Probe>, Error> {
    match read_id_file(SourceKind::Dmi, path)? {
        Some(probe) if is_dmi_garbage(probe.value()) => {
            log::debug!(
                "host-identity: DMI product_uuid {} matches a known vendor-placeholder; \
                 falling through",
                probe.value()
            );
            Ok(None)
        }
        other => Ok(other),
    }
}

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

/// `/etc/hostid` — the glibc 4-byte binary hostid file read by
/// [`gethostid(3)`](https://man7.org/linux/man-pages/man3/gethostid.3.html)
/// and [`hostid(1)`](https://www.gnu.org/software/coreutils/hostid).
///
/// Opt-in only: **not** part of [`crate::sources::default_chain`] or
/// [`crate::sources::network_default_chain`]. On stock Linux distros the
/// file is absent (no `sethostid` has run), so defaulting it would cost
/// every caller a syscall for a near-universal miss. Ship it as a
/// constructible type so operators who know they have `/etc/hostid`
/// (`OpenZFS` hosts, minimal non-systemd images, Red Hat containers that
/// bind-mount `machine-id` but not `hostid`) can push it explicitly.
///
/// # File format
///
/// glibc stores the hostid as four raw bytes in native byte order
/// ([`sethostid(2)`](https://man7.org/linux/man-pages/man2/sethostid.2.html)).
/// Decoded with `u32::from_ne_bytes(...)` and formatted as 8-digit
/// lowercase hex to match `hostid(1)` output.
///
/// # Why we don't call `gethostid(3)`
///
/// When `/etc/hostid` is absent glibc fabricates a value from
/// `gethostname()` → IPv4 lookup. That value is neither stable nor
/// unique and would flow through as identity — actively harmful. This
/// source reads the file directly; absence yields `Ok(None)` so the
/// resolver falls through.
///
/// # Probe behaviour
///
/// - File absent / `PermissionDenied` → `Ok(None)`.
/// - File size ≠ 4 bytes → `Ok(None)` with a `log::debug!` entry
///   (defensive: sheared reads, FreeBSD text-UUID `/etc/hostid`
///   mistakenly placed on Linux).
/// - Value `0x00000000` or `0xffffffff` → `Ok(None)` with a
///   `log::debug!` entry (unset or known-garbage sentinels).
/// - Other I/O error → `Err(Error::Io)`.
/// - Otherwise → `Ok(Some(Probe::new(SourceKind::LinuxHostId, "<hex>")))`.
#[derive(Debug, Clone)]
pub struct LinuxHostIdFile {
    path: PathBuf,
}

impl LinuxHostIdFile {
    /// Read from the standard path (`/etc/hostid`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            path: PathBuf::from("/etc/hostid"),
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

impl Default for LinuxHostIdFile {
    fn default() -> Self {
        Self::new()
    }
}

impl Source for LinuxHostIdFile {
    fn kind(&self) -> SourceKind {
        SourceKind::LinuxHostId
    }
    fn probe(&self) -> Result<Option<Probe>, Error> {
        read_linux_hostid(&self.path)
    }
}

fn read_linux_hostid(path: &Path) -> Result<Option<Probe>, Error> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            log::debug!(
                "host-identity: permission denied reading {}",
                path.display()
            );
            return Ok(None);
        }
        Err(source) => {
            return Err(Error::Io {
                source_kind: SourceKind::LinuxHostId,
                path: PathBuf::from(path),
                source,
            });
        }
    };
    // Read up to five bytes so a file whose size is 4 fills the buffer
    // exactly while a larger file (FreeBSD text UUID etc.) overshoots
    // and is rejected.
    let mut buf = Vec::with_capacity(5);
    file.take(5)
        .read_to_end(&mut buf)
        .map_err(|source| Error::Io {
            source_kind: SourceKind::LinuxHostId,
            path: PathBuf::from(path),
            source,
        })?;
    let Ok(bytes): Result<[u8; 4], _> = buf.as_slice().try_into() else {
        log::debug!(
            "host-identity: /etc/hostid at {} is {} bytes, expected 4; falling through",
            path.display(),
            buf.len(),
        );
        return Ok(None);
    };
    let value = u32::from_ne_bytes(bytes);
    if value == 0 || value == u32::MAX {
        log::debug!(
            "host-identity: /etc/hostid at {} is {value:#010x} (unset/sentinel); falling through",
            path.display()
        );
        return Ok(None);
    }
    Ok(Some(Probe::new(
        SourceKind::LinuxHostId,
        format!("{value:08x}"),
    )))
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

    fn machine_id_probe(kind: SourceKind, body: &str) -> Option<Probe> {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "{body}").unwrap();
        read_machine_id_file(kind, f.path()).unwrap()
    }

    #[test]
    fn machine_id_rejects_whonix_constant() {
        // Removing this entry from MACHINE_ID_DENYLIST must fail this test.
        assert!(
            machine_id_probe(SourceKind::MachineId, "b08dfa6083e7567a1921a715000001fb\n").is_none()
        );
    }

    #[test]
    fn machine_id_rejects_whonix_constant_uppercase() {
        assert!(
            machine_id_probe(SourceKind::MachineId, "B08DFA6083E7567A1921A715000001FB\n").is_none()
        );
    }

    #[test]
    fn machine_id_rejects_oraclelinux_9_constant() {
        assert!(
            machine_id_probe(SourceKind::MachineId, "d495c4b7bb8244639186ef65305fd685\n").is_none()
        );
    }

    #[test]
    fn machine_id_rejects_oraclelinux_8_constant() {
        assert!(
            machine_id_probe(SourceKind::MachineId, "e28a15f597cd4693bb61f1f3e8447cbd\n").is_none()
        );
    }

    #[test]
    fn machine_id_rejects_jrei_systemd_debian_constant() {
        assert!(
            machine_id_probe(SourceKind::MachineId, "4c010dc413ad444698de6ee4677331b9\n").is_none()
        );
    }

    #[test]
    fn machine_id_rejects_jrei_systemd_ubuntu_constant() {
        assert!(
            machine_id_probe(SourceKind::MachineId, "a7570853ab864bbbbfc8c54b14eeaf8f\n").is_none()
        );
    }

    #[test]
    fn machine_id_rejects_geerlingguy_ansible_ubuntu_constant() {
        assert!(
            machine_id_probe(SourceKind::MachineId, "5b4bb40898b2416087b6224f176978fb\n").is_none()
        );
    }

    #[test]
    fn machine_id_rejects_geerlingguy_ansible_debian_constant() {
        assert!(
            machine_id_probe(SourceKind::MachineId, "3948e4ca87b64871b31c9a49920b9834\n").is_none()
        );
    }

    #[test]
    fn machine_id_rejects_geerlingguy_ansible_rocky_constant() {
        assert!(
            machine_id_probe(SourceKind::MachineId, "835aa90928e143e3ae09efcd0c5cb118\n").is_none()
        );
    }

    #[test]
    fn machine_id_rejects_all_zero_hex32() {
        assert!(machine_id_probe(SourceKind::MachineId, &"0".repeat(32)).is_none());
    }

    #[test]
    fn machine_id_rejects_all_same_nibble_hex32() {
        assert!(machine_id_probe(SourceKind::MachineId, &"a".repeat(32)).is_none());
        assert!(machine_id_probe(SourceKind::MachineId, &"F".repeat(32)).is_none());
    }

    #[test]
    fn machine_id_accepts_plausible_real_value() {
        let probe =
            machine_id_probe(SourceKind::MachineId, "4c4c4544003957108052b4c04f384833\n").unwrap();
        assert_eq!(probe.value(), "4c4c4544003957108052b4c04f384833");
    }

    #[test]
    fn machine_id_filter_trims_whitespace_before_matching() {
        // Confirms the filter composes with classify's trim.
        assert!(
            machine_id_probe(
                SourceKind::MachineId,
                "  b08dfa6083e7567a1921a715000001fb  \n\t"
            )
            .is_none()
        );
    }

    #[test]
    fn dbus_machine_id_rejects_whonix_constant() {
        // Confirms the filter is wired into DbusMachineIdFile too.
        assert!(
            machine_id_probe(
                SourceKind::DbusMachineId,
                "b08dfa6083e7567a1921a715000001fb\n"
            )
            .is_none()
        );
    }

    #[test]
    fn hostid_accepts_all_zero_hex32() {
        // Negative control: the machine-id filter must not leak into
        // unrelated sources via read_id_file.
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "{}", "0".repeat(32)).unwrap();
        let probe = read_id_file(SourceKind::LinuxHostId, f.path())
            .unwrap()
            .unwrap();
        assert_eq!(probe.value(), "0".repeat(32));
    }

    #[test]
    fn machine_id_file_probe_applies_filter() {
        // End-to-end: MachineIdFile's Source::probe() (via the
        // file_source! macro) must route through read_machine_id_file.
        // Guards against regressions pointing the macro back at
        // read_id_file.
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "b08dfa6083e7567a1921a715000001fb").unwrap();
        let probe = MachineIdFile::at(f.path()).probe().unwrap();
        assert!(probe.is_none());
    }

    #[test]
    fn dbus_machine_id_file_probe_applies_filter() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "b08dfa6083e7567a1921a715000001fb").unwrap();
        let probe = DbusMachineIdFile::at(f.path()).probe().unwrap();
        assert!(probe.is_none());
    }

    #[test]
    fn is_all_same_nibble_hex32_rejects_short_values() {
        // Gate at exactly 32 chars so short non-hex32 strings pass through.
        assert!(!is_all_same_nibble_hex32("aaa"));
        assert!(!is_all_same_nibble_hex32(""));
        assert!(!is_all_same_nibble_hex32(&"a".repeat(31)));
        assert!(!is_all_same_nibble_hex32(&"a".repeat(33)));
    }

    #[test]
    fn is_all_same_nibble_hex32_rejects_non_hex() {
        assert!(!is_all_same_nibble_hex32(&"z".repeat(32)));
    }

    fn dmi_tempfile(body: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "{body}").unwrap();
        f
    }

    fn dmi_probe(body: &str) -> Option<Probe> {
        let f = dmi_tempfile(body);
        read_dmi_file(f.path()).unwrap()
    }

    #[test]
    fn dmi_rejects_all_zero_uuid() {
        assert!(dmi_probe("00000000-0000-0000-0000-000000000000\n").is_none());
    }

    #[test]
    fn dmi_rejects_all_f_uuid_lower() {
        assert!(dmi_probe("ffffffff-ffff-ffff-ffff-ffffffffffff\n").is_none());
    }

    #[test]
    fn dmi_rejects_all_f_uuid_upper() {
        assert!(dmi_probe("FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF\n").is_none());
    }

    #[test]
    fn dmi_rejects_all_same_nibble_1() {
        assert!(dmi_probe("11111111-1111-1111-1111-111111111111\n").is_none());
    }

    #[test]
    fn dmi_rejects_all_same_nibble_a() {
        assert!(dmi_probe("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\n").is_none());
    }

    #[test]
    fn dmi_rejects_supermicro_ami_placeholder() {
        // Removing this entry from DMI_PLACEHOLDER_UUIDS must fail this
        // test — deliberate regression coverage.
        assert!(dmi_probe("03000200-0400-0500-0006-000700080009\n").is_none());
    }

    #[test]
    fn dmi_rejects_supermicro_ami_placeholder_uppercase() {
        assert!(
            dmi_probe(
                "03000200-0400-0500-0006-000700080009"
                    .to_ascii_uppercase()
                    .as_str()
            )
            .is_none()
        );
    }

    #[test]
    fn dmi_rejects_garbage_with_trailing_whitespace() {
        // Confirms the filter composes with classify's trim.
        assert!(dmi_probe("  00000000-0000-0000-0000-000000000000  \n\t").is_none());
    }

    #[test]
    fn dmi_accepts_plausible_real_uuid() {
        let probe = dmi_probe("4c4c4544-0039-5710-8052-b4c04f384833\n").unwrap();
        assert_eq!(probe.value(), "4c4c4544-0039-5710-8052-b4c04f384833");
    }

    #[test]
    fn dmi_accepts_non_uuid_shape() {
        // The 32-hex-digit gate in is_all_same_nibble_uuid must not
        // false-positively reject short non-UUID values.
        let probe = dmi_probe("abcdef\n").unwrap();
        assert_eq!(probe.value(), "abcdef");
    }

    #[test]
    fn machine_id_file_accepts_hyphenated_all_zero_uuid() {
        // The machine-id filter's hex32 predicate deliberately requires
        // exactly 32 hex digits with no hyphens (per the systemd
        // machine-id format). A hyphenated all-zero UUID is not a valid
        // machine-id shape but must not be rejected here — it would be
        // the caller's job to write a correctly-shaped file.
        let probe = machine_id_probe(
            SourceKind::MachineId,
            "00000000-0000-0000-0000-000000000000\n",
        )
        .unwrap();
        assert_eq!(probe.value(), "00000000-0000-0000-0000-000000000000");
    }

    fn write_hostid(bytes: &[u8]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        f
    }

    #[test]
    fn linux_hostid_reads_native_endian_bytes() {
        // `hostid(1)` prints `u32::from_ne_bytes(file_bytes)` formatted
        // as 8-digit lowercase hex. Mirror that contract for both
        // endiannesses so the test is honest on BE targets too.
        let file_bytes = [0x8f, 0x8f, 0x98, 0x4f];
        let expected = format!("{:08x}", u32::from_ne_bytes(file_bytes));
        let f = write_hostid(&file_bytes);
        let probe = read_linux_hostid(f.path()).unwrap().unwrap();
        assert_eq!(probe.kind(), SourceKind::LinuxHostId);
        assert_eq!(probe.value(), expected);
    }

    #[test]
    fn linux_hostid_pads_small_values_to_eight_hex_digits() {
        // Pin the `{:08x}` width specifier: a small value like 0x42
        // must render as "00000042", matching `hostid(1)`'s `%08x`.
        // Build the file bytes from the target-native u32 so the test
        // is honest on both endiannesses.
        let file_bytes = 0x0000_0042_u32.to_ne_bytes();
        let f = write_hostid(&file_bytes);
        let probe = read_linux_hostid(f.path()).unwrap().unwrap();
        assert_eq!(probe.value(), "00000042");
    }

    #[test]
    fn linux_hostid_missing_is_none() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("absent");
        assert!(read_linux_hostid(&missing).unwrap().is_none());
    }

    #[test]
    fn linux_hostid_wrong_size_too_small_is_none() {
        let f = write_hostid(&[0x01, 0x02, 0x03]);
        assert!(read_linux_hostid(f.path()).unwrap().is_none());
    }

    #[test]
    fn linux_hostid_wrong_size_too_large_is_none() {
        // FreeBSD ships a text UUID at /etc/hostid — longer than 4
        // bytes. Defensive short-circuit so a FreeBSD file mistakenly
        // placed on Linux falls through.
        let f = write_hostid(b"4f988f8f-0000-0000-0000-000000000000\n");
        assert!(read_linux_hostid(f.path()).unwrap().is_none());
    }

    #[test]
    fn linux_hostid_empty_is_none() {
        let f = write_hostid(&[]);
        assert!(read_linux_hostid(f.path()).unwrap().is_none());
    }

    #[test]
    fn linux_hostid_rejects_all_zero() {
        let f = write_hostid(&[0, 0, 0, 0]);
        assert!(read_linux_hostid(f.path()).unwrap().is_none());
    }

    #[test]
    fn linux_hostid_rejects_all_ff() {
        let f = write_hostid(&[0xff, 0xff, 0xff, 0xff]);
        assert!(read_linux_hostid(f.path()).unwrap().is_none());
    }

    #[test]
    fn linux_hostid_reports_io_error_for_directory() {
        let dir = TempDir::new().unwrap();
        let err = read_linux_hostid(dir.path())
            .expect_err("reading a directory must surface as Error::Io");
        match err {
            Error::Io {
                path, source_kind, ..
            } => {
                assert_eq!(path, dir.path());
                assert_eq!(source_kind, SourceKind::LinuxHostId);
            }
            other => panic!("expected Io, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn linux_hostid_permission_denied_is_none() {
        use std::os::unix::fs::PermissionsExt;
        use std::path::PathBuf;

        struct PermGuard(PathBuf);
        impl Drop for PermGuard {
            fn drop(&mut self) {
                let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o600));
            }
        }

        if nix_is_root() {
            return;
        }
        let f = write_hostid(&[0x01, 0x02, 0x03, 0x04]);
        std::fs::set_permissions(f.path(), std::fs::Permissions::from_mode(0o000)).unwrap();
        let _guard = PermGuard(f.path().to_path_buf());
        assert!(read_linux_hostid(f.path()).unwrap().is_none());
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
