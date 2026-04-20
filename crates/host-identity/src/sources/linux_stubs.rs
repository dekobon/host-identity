//! Non-Linux stubs for the Linux source types. Each returns `Ok(None)` so
//! they can appear in a cross-platform resolver chain without `cfg` guards
//! at the call site.

use crate::source::SourceKind;
use crate::sources::stub_macros::path_stub;

path_stub!(
    /// `/etc/machine-id` source (no-op on non-Linux targets).
    MachineIdFile,
    SourceKind::MachineId,
    "/etc/machine-id"
);
path_stub!(
    /// `/var/lib/dbus/machine-id` source (no-op on non-Linux targets).
    DbusMachineIdFile,
    SourceKind::DbusMachineId,
    "/var/lib/dbus/machine-id"
);
path_stub!(
    /// SMBIOS DMI UUID source (no-op on non-Linux targets).
    DmiProductUuid,
    SourceKind::Dmi,
    "/sys/class/dmi/id/product_uuid"
);
path_stub!(
    /// glibc `/etc/hostid` binary source (no-op on non-Linux targets).
    LinuxHostIdFile,
    SourceKind::LinuxHostId,
    "/etc/hostid"
);
