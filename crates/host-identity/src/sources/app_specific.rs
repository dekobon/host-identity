//! App-specific derivation wrapper.
//!
//! Wraps an inner [`Source`] and replaces its probe value with an
//! `HMAC-SHA256`-derived UUID keyed on the inner value and messaged with
//! a caller-supplied `app_id`. Two apps on the same host that each
//! wrap the same inner source with different `app_id`s get
//! uncorrelatable IDs, and the raw inner value is never exposed on the
//! resolved [`crate::HostId`].
//!
//! This is the generalised form of systemd's
//! [`sd_id128_get_machine_app_specific()`](https://man.archlinux.org/man/sd_id128_get_machine_app_specific.3.en)
//! — the privacy property (stable + uncorrelatable across apps + does
//! not leak the raw key) applies to every stable machine key this
//! crate abstracts, not just `/etc/machine-id`.
//!
//! # Construction
//!
//! `HMAC-SHA256(key = inner_value_bytes, msg = app_id)` truncated to
//! the first 16 bytes, with the RFC 9562 version-4 and variant-10 bits
//! forced, formatted as a hyphenated UUID string (the same shape as
//! [`crate::sources::DmiProductUuid`], [`crate::sources::IoPlatformUuid`],
//! [`crate::sources::WindowsMachineGuid`], and [`crate::sources::KenvSmbios`]).
//!
//! Because the output is a UUID string, [`crate::Wrap::Passthrough`]
//! round-trips the probe unchanged, and the default
//! [`crate::Wrap::UuidV5Namespaced`] re-hashes it for crate-namespace
//! separation the same way it does for the other UUID-native sources
//! — not double-hashing an already-hashed 256-bit value.
//!
//! # systemd byte-compat
//!
//! Byte-compatibility with
//! `systemd-id128 machine-id --app-specific=<app-uuid>` holds **only**
//! when `app_id` is exactly 16 bytes derived from the same UUID
//! systemd would accept on its CLI, and the inner source is
//! [`crate::sources::MachineIdFile`] (raw 32-hex machine-id bytes as
//! the HMAC key). Rust callers are free to pass arbitrary `&[u8]`
//! app-ids — they just forfeit the systemd byte-compat side effect.
//!
//! # Privacy caveats
//!
//! - The inner source's raw value acts as the HMAC key — treat it as
//!   sensitive. The `hmac` crate holds its own internal copy of the
//!   key which this crate cannot reach; the `app_id` buffer is
//!   zeroized on drop as a best-effort mitigation.
//! - Wrapping an inner source whose raw value is already public
//!   (cloud instance IDs visible in consoles, Kubernetes pod UIDs
//!   readable via the API server) adds no privacy — the input isn't
//!   secret in the first place.
//! - The derived ID is an identifier, **not** key material. Callers
//!   must not use it as a cryptographic key.

use std::fmt;

use hmac::{Hmac, Mac};
use sha2::Sha256;
use uuid::Uuid;
use zeroize::Zeroize;

use crate::error::Error;
use crate::source::{Probe, Source, SourceKind};

type HmacSha256 = Hmac<Sha256>;

/// Wrapper source that HMACs the inner source's probe value with a
/// caller-supplied `app_id`, emitting a UUID-shaped probe.
///
/// See the [module-level docs](self) for the construction, the
/// `Wrap::Passthrough` contract, and the systemd byte-compat caveat.
///
/// # Example
///
/// ```no_run
/// use host_identity::sources::{AppSpecific, MachineIdFile};
/// use host_identity::{Resolver, Source};
///
/// let wrapped = AppSpecific::new(
///     MachineIdFile::default(),
///     b"com.example.telemetry".to_vec(),
/// );
/// let id = Resolver::new().push(wrapped).resolve()?;
/// # Ok::<(), host_identity::Error>(())
/// ```
pub struct AppSpecific<S: Source> {
    inner: S,
    app_id: Vec<u8>,
    label: &'static str,
}

impl<S: Source> AppSpecific<S> {
    /// Wrap `inner` so its probe value is derived with `app_id`.
    ///
    /// `app_id` is not secret — privacy comes from not leaking the
    /// inner source's raw value, not from `app_id` secrecy. Pick a
    /// stable byte string that identifies your application (reverse
    /// DNS, a random UUID, a git SHA — whichever is convenient and
    /// stable across your deployment).
    ///
    /// # Label interning
    ///
    /// The composed provenance label `app-specific:<inner-id>` is
    /// allocated once per `AppSpecific::new` call and leaked for the
    /// program's lifetime so that [`Source::kind`] can return it
    /// through [`SourceKind::Custom`]. Construction rate is bounded in
    /// practice (sources are built once per resolver), so the leak is
    /// accepted in exchange for not holding a process-wide lock on a
    /// string interner.
    #[must_use]
    pub fn new(inner: S, app_id: impl Into<Vec<u8>>) -> Self {
        let label: &'static str =
            Box::leak(format!("app-specific:{}", inner.kind().as_str()).into_boxed_str());
        Self {
            inner,
            app_id: app_id.into(),
            label,
        }
    }
}

impl<S: Source> fmt::Debug for AppSpecific<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppSpecific")
            .field("inner", &self.inner.kind())
            .field("app_id_len", &self.app_id.len())
            .finish_non_exhaustive()
    }
}

impl<S: Source> Drop for AppSpecific<S> {
    fn drop(&mut self) {
        self.app_id.zeroize();
    }
}

impl<S: Source> Source for AppSpecific<S> {
    fn kind(&self) -> SourceKind {
        SourceKind::Custom(self.label)
    }

    fn probe(&self) -> Result<Option<Probe>, Error> {
        let Some(probe) = self.inner.probe()? else {
            return Ok(None);
        };
        let (_inner_kind, raw) = probe.into_parts();
        let uuid = derive_app_specific_uuid(raw.as_bytes(), &self.app_id);
        Ok(Some(Probe::new(self.kind(), uuid.hyphenated().to_string())))
    }
}

/// Compute `HMAC-SHA256(key = raw, msg = app_id)`, truncate to 16 bytes,
/// force the UUID v4 version and variant-10 bits, and return the UUID.
fn derive_app_specific_uuid(raw: &[u8], app_id: &[u8]) -> Uuid {
    let mut mac = HmacSha256::new_from_slice(raw).expect("HMAC-SHA256 accepts keys of any length");
    mac.update(app_id);
    let digest = mac.finalize().into_bytes();
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&digest[..16]);
    buf[6] = (buf[6] & 0x0F) | 0x40;
    buf[8] = (buf[8] & 0x3F) | 0x80;
    let uuid = Uuid::from_bytes(buf);
    buf.zeroize();
    uuid
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{Probe, Source, SourceKind};
    use crate::wrap::Wrap;

    /// Test stub: returns whatever `value`/`kind` it was built with.
    #[derive(Debug)]
    struct Stub {
        kind: SourceKind,
        result: Result<Option<String>, &'static str>,
    }

    impl Stub {
        fn ok(kind: SourceKind, v: &str) -> Self {
            Self {
                kind,
                result: Ok(Some(v.to_owned())),
            }
        }
        fn none(kind: SourceKind) -> Self {
            Self {
                kind,
                result: Ok(None),
            }
        }
        fn err(kind: SourceKind, msg: &'static str) -> Self {
            Self {
                kind,
                result: Err(msg),
            }
        }
    }

    impl Source for Stub {
        fn kind(&self) -> SourceKind {
            self.kind
        }
        fn probe(&self) -> Result<Option<Probe>, Error> {
            match &self.result {
                Ok(Some(v)) => Ok(Some(Probe::new(self.kind, v.clone()))),
                Ok(None) => Ok(None),
                Err(msg) => Err(Error::Malformed {
                    source_kind: self.kind,
                    reason: (*msg).to_owned(),
                }),
            }
        }
    }

    fn probe_value(s: &impl Source) -> String {
        s.probe().unwrap().unwrap().value().to_owned()
    }

    #[test]
    fn output_is_a_valid_version4_uuid() {
        let wrapped = AppSpecific::new(
            Stub::ok(SourceKind::MachineId, "abcdef0123456789abcdef0123456789"),
            b"com.example.test".to_vec(),
        );
        let v = probe_value(&wrapped);
        let parsed = Uuid::parse_str(&v).expect("valid UUID");
        assert_eq!(parsed.get_version_num(), 4);
        let variant_byte = parsed.as_bytes()[8];
        assert_eq!(variant_byte & 0xC0, 0x80, "variant must be 10xx");
        // Locks hyphenated 8-4-4-4-12 lowercase shape.
        let re_parts: Vec<_> = v.split('-').collect();
        assert_eq!(
            re_parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            vec![8, 4, 4, 4, 12]
        );
        assert!(v.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
    }

    #[test]
    fn construction_matches_manual_hmac_sha256() {
        // Regression lock: the wrapper is a thin shim over
        // `HMAC-SHA256(key=raw, msg=app_id)` with v4/variant-10 bits
        // forced. This test recomputes the construction by hand and
        // asserts equality. A failure means the derivation function
        // drifted from the documented construction — review before
        // updating.
        //
        // The same construction yields byte-compat with
        // `systemd-id128 machine-id --app-specific=<APP-UUID>` when
        // `raw` is the /etc/machine-id bytes and `app_id` is the
        // 16-byte UUID systemd would accept. The test uses a fixture
        // machine-id and a 16-byte app-id derived from the UUID
        // "a2b16c2f-0fa0-4d32-b3c3-1ee8c22c0b7e" to exercise that
        // shape, but does not depend on `systemd-id128` being
        // installed.
        let raw = b"abcdef0123456789abcdef0123456789";
        let app_id: [u8; 16] = [
            0xa2, 0xb1, 0x6c, 0x2f, 0x0f, 0xa0, 0x4d, 0x32, 0xb3, 0xc3, 0x1e, 0xe8, 0xc2, 0x2c,
            0x0b, 0x7e,
        ];
        let got = derive_app_specific_uuid(raw, &app_id);
        let mut mac = HmacSha256::new_from_slice(raw).unwrap();
        mac.update(&app_id);
        let digest = mac.finalize().into_bytes();
        let mut buf = [0u8; 16];
        buf.copy_from_slice(&digest[..16]);
        buf[6] = (buf[6] & 0x0F) | 0x40;
        buf[8] = (buf[8] & 0x3F) | 0x80;
        assert_eq!(got, Uuid::from_bytes(buf));
        assert_eq!(got.get_version_num(), 4);
    }

    #[test]
    fn determinism_over_many_iterations() {
        let wrapped = AppSpecific::new(
            Stub::ok(SourceKind::MachineId, "raw-value"),
            b"app".to_vec(),
        );
        let first = probe_value(&wrapped);
        for _ in 0..100 {
            assert_eq!(probe_value(&wrapped), first);
        }
    }

    #[test]
    fn different_app_ids_produce_different_outputs() {
        let a = AppSpecific::new(Stub::ok(SourceKind::MachineId, "x"), b"app-1".to_vec());
        let b = AppSpecific::new(Stub::ok(SourceKind::MachineId, "x"), b"app-2".to_vec());
        assert_ne!(probe_value(&a), probe_value(&b));
    }

    #[test]
    fn different_inner_values_produce_different_outputs() {
        let a = AppSpecific::new(Stub::ok(SourceKind::MachineId, "x"), b"app".to_vec());
        let b = AppSpecific::new(Stub::ok(SourceKind::MachineId, "y"), b"app".to_vec());
        assert_ne!(probe_value(&a), probe_value(&b));
    }

    #[test]
    fn passthrough_wrap_round_trips_the_probe() {
        let wrapped = AppSpecific::new(Stub::ok(SourceKind::MachineId, "raw"), b"app".to_vec());
        let v = probe_value(&wrapped);
        let roundtrip = Wrap::Passthrough.apply(&v).expect("UUID-shaped");
        assert_eq!(roundtrip, Uuid::parse_str(&v).unwrap());
    }

    #[test]
    fn default_wrap_is_stable() {
        let wrapped = AppSpecific::new(Stub::ok(SourceKind::MachineId, "raw"), b"app".to_vec());
        let v1 = probe_value(&wrapped);
        let v2 = probe_value(&wrapped);
        assert_eq!(
            Wrap::UuidV5Namespaced.apply(&v1),
            Wrap::UuidV5Namespaced.apply(&v2),
        );
    }

    #[test]
    fn scope_label_is_app_specific_prefixed() {
        let wrapped = AppSpecific::new(Stub::ok(SourceKind::MachineId, "raw"), b"app".to_vec());
        assert_eq!(wrapped.kind().as_str(), "app-specific:machine-id");
        let probe = wrapped.probe().unwrap().unwrap();
        assert_eq!(probe.kind().as_str(), "app-specific:machine-id");
    }

    #[test]
    fn inner_none_is_passed_through() {
        let wrapped = AppSpecific::new(Stub::none(SourceKind::MachineId), b"app".to_vec());
        assert!(wrapped.probe().unwrap().is_none());
    }

    #[test]
    fn inner_err_is_passed_through() {
        let wrapped = AppSpecific::new(Stub::err(SourceKind::MachineId, "boom"), b"app".to_vec());
        let err = wrapped.probe().expect_err("error must propagate");
        // The wrapper must surface the inner source's provenance and
        // reason verbatim rather than re-labelling as app-specific.
        match err {
            Error::Malformed {
                source_kind,
                reason,
            } => {
                assert_eq!(source_kind, SourceKind::MachineId);
                assert_eq!(reason, "boom");
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn empty_inputs_do_not_panic_and_produce_valid_uuids() {
        // Empty raw, empty app_id.
        let wrapped = AppSpecific::new(Stub::ok(SourceKind::MachineId, ""), Vec::<u8>::new());
        let v = probe_value(&wrapped);
        let parsed = Uuid::parse_str(&v).expect("valid UUID even with empty inputs");
        assert_eq!(parsed.get_version_num(), 4);
    }
}
