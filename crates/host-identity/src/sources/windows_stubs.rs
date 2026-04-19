//! Non-Windows stub. Returns `Ok(None)` so the type can appear portably.

use crate::source::SourceKind;
use crate::sources::stub_macros::unit_stub;

unit_stub!(
    /// Windows `MachineGuid` registry source (no-op on non-Windows targets).
    WindowsMachineGuid,
    SourceKind::WindowsMachineGuid
);
