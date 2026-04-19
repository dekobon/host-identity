//! Non-FreeBSD stubs. Return `Ok(None)` so the types can appear portably.

use crate::source::SourceKind;
use crate::sources::stub_macros::{path_stub, unit_stub};

path_stub!(
    /// FreeBSD `/etc/hostid` source (no-op on non-FreeBSD targets).
    FreeBsdHostIdFile,
    SourceKind::FreeBsdHostId,
    "/etc/hostid"
);

unit_stub!(
    /// FreeBSD `kenv smbios.system.uuid` source (no-op on non-FreeBSD targets).
    KenvSmbios,
    SourceKind::KenvSmbios
);
