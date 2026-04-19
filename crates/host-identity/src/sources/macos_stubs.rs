//! Non-macOS stub. Returns `Ok(None)` so the type can appear portably.

use crate::source::SourceKind;
use crate::sources::stub_macros::unit_stub;

unit_stub!(
    /// macOS `IOPlatformUUID` source (no-op on non-macOS targets).
    IoPlatformUuid,
    SourceKind::IoPlatformUuid
);
