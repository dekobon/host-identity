//! Non-illumos/Solaris stub. Returns `Ok(None)` so the type can appear portably.

use crate::source::SourceKind;
use crate::sources::stub_macros::unit_stub;

unit_stub!(
    /// illumos / Solaris `hostid(1)` source (no-op on other targets).
    IllumosHostId,
    SourceKind::IllumosHostId
);
