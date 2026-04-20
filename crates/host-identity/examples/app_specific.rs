//! Per-application, privacy-preserving host identity with [`AppSpecific`].
//!
//! [`AppSpecific`] wraps any inner [`Source`] and emits
//! `HMAC-SHA256(key = inner_probe_bytes, msg = app_id)` truncated to a
//! UUID, so two apps on the same host see uncorrelatable IDs and the
//! raw inner value never appears on the resolved `HostId`. This is the
//! generalisation of systemd's
//! [`sd_id128_get_machine_app_specific(3)`](https://man.archlinux.org/man/sd_id128_get_machine_app_specific.3.en)
//! — handing a raw `/etc/machine-id` to telemetry is exactly what
//! [`machine-id(5)`](https://www.freedesktop.org/software/systemd/man/machine-id.html)
//! warns against.
//!
//! ```bash
//! cargo run --example app_specific
//! ```
//!
//! Requires `/etc/machine-id` to be present and populated (standard on
//! modern Linux; also present on some FreeBSD and bind-mounted
//! containers). On hosts without it the example exits with the
//! resolver's error — swap [`MachineIdFile`] for the platform default
//! (`DmiProductUuid`, `IoPlatformUuid`, `WindowsMachineGuid`, …) to
//! demo elsewhere.
//!
//! [`AppSpecific`]: host_identity::sources::AppSpecific
//! [`Source`]: host_identity::Source

use host_identity::sources::{AppSpecific, MachineIdFile};
use host_identity::{Error, HostId, Resolver};

const APP_ID: &str = "com.example.telemetry";
const OTHER_APP_ID: &str = "com.example.crash-reporter";

fn main() -> Result<(), Error> {
    let id = resolve_for(APP_ID)?;
    println!("app_id  = {APP_ID}");
    println!("uuid    = {id}");
    println!("source  = {}", id.source());

    // Determinism: a second resolve with the same `app_id` returns the
    // same UUID. Cross-process stability follows from `/etc/machine-id`
    // being stable across reboots — not demonstrated here.
    let again = resolve_for(APP_ID)?;
    assert_eq!(id, again);
    println!("stable  = deterministic (re-resolved with same app_id)");

    // Uncorrelatability: a different `app_id` yields a different UUID,
    // even though the inner machine-id is unchanged. Note also that
    // `id.source()` reports `app-specific:machine-id` — the raw
    // machine-id never appears on `HostId`.
    let other = resolve_for(OTHER_APP_ID)?;
    assert_ne!(id, other);
    println!("app_id  = {OTHER_APP_ID}");
    println!("uuid    = {other}");

    Ok(())
}

fn resolve_for(app_id: &str) -> Result<HostId, Error> {
    Resolver::new()
        .push(AppSpecific::new(MachineIdFile::new(), app_id.as_bytes()))
        .resolve()
}
