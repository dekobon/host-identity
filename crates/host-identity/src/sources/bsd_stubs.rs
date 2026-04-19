//! Non-NetBSD/OpenBSD stub. Returns `Ok(None)` so the type can appear portably.

use crate::source::SourceKind;
use crate::sources::stub_macros::unit_stub;

unit_stub!(
    /// `sysctl kern.hostid` source (no-op on non-BSD targets).
    SysctlKernHostId,
    SourceKind::BsdKernHostId
);
