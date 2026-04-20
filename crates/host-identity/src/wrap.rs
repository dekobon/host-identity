//! Strategies for wrapping a raw identifier into a [`uuid::Uuid`].
//!
//! Name-based UUID generation follows
//! [RFC 9562 § 5.3 (`UUIDv3`, MD5)](https://datatracker.ietf.org/doc/html/rfc9562#name-uuid-version-3)
//! and [§ 5.5 (`UUIDv5`, SHA-1)](https://datatracker.ietf.org/doc/html/rfc9562#name-uuid-version-5),
//! which obsoleted [RFC 4122](https://datatracker.ietf.org/doc/html/rfc4122).
//! RFC 9562 recommends `UUIDv5` over `UUIDv3` for new work; this crate exposes
//! both and defaults to `UUIDv5`. The hashing is performed by the
//! [`uuid`](https://docs.rs/uuid) crate's `new_v5` / `new_v3` constructors.

use uuid::Uuid;

/// Namespace used for the default UUID v5 wrap strategy.
///
/// Fixed for the life of the crate so a given raw identifier always maps to
/// the same UUID. Chosen randomly; not shared with any other tool, which is
/// the point — two tools wrapping the same machine-id under different
/// namespaces produce different UUIDs and will not collide.
pub const DEFAULT_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6f, 0x63, 0x1b, 0x9a, 0x2d, 0x4c, 0x5e, 0x11, 0x9b, 0x21, 0x3f, 0x8a, 0xc0, 0x7e, 0x44, 0x21,
]);

/// How the raw identifier produced by a [`crate::Source`] is turned into a
/// [`uuid::Uuid`].
///
/// Pick one with [`crate::Resolver::with_wrap`]. The default
/// ([`Wrap::UuidV5Namespaced`]) is the right choice for new code; the other
/// variants exist for specific interop scenarios.
///
/// | Variant                 | When to use                                                                                                       |
/// | ----------------------- | ----------------------------------------------------------------------------------------------------------------- |
/// | [`UuidV5Namespaced`]    | Default. Strongest collision resistance; rehashes under a private namespace so two tools sharing a raw source cannot collide. |
/// | [`UuidV5With`]          | You want v5 hashing but need the wrapped UUID to live in a namespace already used by another system.              |
/// | [`UuidV3Nil`]           | Wire-compatible with the legacy Go derivation `uuid.NewMD5(uuid.Nil, raw)`. Interop only; prefer v5 otherwise.    |
/// | [`Passthrough`]         | The source already yields a UUID and you want *that exact UUID* to survive unchanged (e.g. match another agent). |
///
/// All deterministic: the same raw input always produces the same UUID.
///
/// [`UuidV5Namespaced`]: Wrap::UuidV5Namespaced
/// [`UuidV5With`]: Wrap::UuidV5With
/// [`UuidV3Nil`]: Wrap::UuidV3Nil
/// [`Passthrough`]: Wrap::Passthrough
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Wrap {
    /// UUID v5 (SHA-1) under the crate's [`DEFAULT_NAMESPACE`]. Default;
    /// strongest collision resistance of the deterministic options and the
    /// right choice unless you have a concrete interop requirement.
    ///
    /// Rehashes the raw value even when the source already yields a UUID
    /// (DMI `product_uuid`, macOS `IOPlatformUUID`, Windows `MachineGuid`,
    /// SMBIOS). That is intentional: it prevents two tools that share a
    /// raw source (e.g. two agents both reading `/etc/machine-id`) from
    /// emitting colliding IDs. Use [`Wrap::Passthrough`] when you
    /// explicitly want the source's own UUID to survive unchanged, or
    /// [`Wrap::UuidV5With`] when you need a different namespace.
    #[default]
    UuidV5Namespaced,

    /// UUID v5 (SHA-1) under a caller-supplied namespace. Same algorithm
    /// as [`Wrap::UuidV5Namespaced`] with a different namespace constant.
    ///
    /// Use when another system in your stack already hashes identifiers
    /// under a well-known namespace (e.g. a product-wide DNS namespace)
    /// and you want this crate's output to sit in that same space so IDs
    /// cross-correlate. If you don't have such a namespace, stick with
    /// the default.
    UuidV5With(Uuid),

    /// UUID v3 (MD5) under the nil namespace — wire-compatible with the
    /// legacy Go derivation `uuid.NewMD5(uuid.Nil, raw)`.
    ///
    /// Use only for interop with existing pipelines that already produced
    /// IDs this way; MD5 has no security relevance here, but RFC 9562
    /// recommends v5 over v3 for new work and so does this crate.
    UuidV3Nil,

    /// Parse the raw value directly as a UUID, with no hashing.
    ///
    /// Use when the source already yields a UUID string (DMI
    /// `product_uuid`, macOS `IOPlatformUUID`, Windows `MachineGuid`,
    /// `kenv smbios.system.uuid`, container IDs, Kubernetes pod UIDs)
    /// and you want *that exact UUID* to survive unchanged — for example,
    /// to match the ID another agent on the same host already reports.
    ///
    /// Returns `None` (surfaced as [`crate::Error::Malformed`] from the
    /// resolver) when the raw value is not a parseable UUID, so this
    /// strategy is unsafe to pair with sources that emit arbitrary
    /// strings (e.g. `HOST_IDENTITY=my-server`).
    ///
    /// Accepts every form [`uuid::Uuid::parse_str`] accepts — hyphenated
    /// (`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`), simple (no hyphens),
    /// braced (`{…}`), and the RFC-9562 `urn:uuid:…` form. The parsed
    /// UUID is returned in canonical form regardless of the input shape.
    Passthrough,
}

impl Wrap {
    /// Apply this strategy to a raw identifier.
    ///
    /// Returns `None` for [`Wrap::Passthrough`] when the raw value cannot be
    /// parsed as a UUID. All other strategies always succeed.
    #[must_use]
    pub fn apply(self, raw: &str) -> Option<Uuid> {
        match self {
            Self::UuidV5Namespaced => Some(Uuid::new_v5(&DEFAULT_NAMESPACE, raw.as_bytes())),
            Self::UuidV5With(ns) => Some(Uuid::new_v5(&ns, raw.as_bytes())),
            Self::UuidV3Nil => Some(Uuid::new_v3(&Uuid::nil(), raw.as_bytes())),
            Self::Passthrough => Uuid::parse_str(raw).ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v5_default_is_deterministic() {
        let a = Wrap::UuidV5Namespaced.apply("host-x").unwrap();
        let b = Wrap::UuidV5Namespaced.apply("host-x").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn v5_distinct_namespaces_produce_distinct_uuids() {
        let ns = Uuid::from_bytes([1; 16]);
        let a = Wrap::UuidV5Namespaced.apply("host-x").unwrap();
        let b = Wrap::UuidV5With(ns).apply("host-x").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn passthrough_roundtrips_valid_uuid() {
        let uuid = "12345678-1234-1234-1234-123456789abc";
        assert_eq!(Wrap::Passthrough.apply(uuid), Uuid::parse_str(uuid).ok());
    }

    #[test]
    fn passthrough_rejects_non_uuid() {
        assert_eq!(Wrap::Passthrough.apply("not-a-uuid"), None);
    }

    #[test]
    fn v3_nil_matches_go_legacy_derivation() {
        // Wire-compat contract with agent-go's `uuid.NewMD5(uuid.Nil, raw)`.
        // Must equal the stdlib Uuid::new_v3 under the nil namespace.
        let expected = Uuid::new_v3(&Uuid::nil(), b"host-x");
        assert_eq!(Wrap::UuidV3Nil.apply("host-x"), Some(expected));
    }

    #[test]
    fn v3_nil_is_deterministic() {
        let a = Wrap::UuidV3Nil.apply("host-x").unwrap();
        let b = Wrap::UuidV3Nil.apply("host-x").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn non_passthrough_strategies_always_return_some() {
        // Locks the "All other strategies always succeed" contract
        // documented on `Wrap::apply`. Empty, whitespace-only, and
        // long pathological inputs must never produce `None`.
        let ns = Uuid::from_bytes([1; 16]);
        let inputs = ["", "   \n", &"a".repeat(10_000)];
        for input in inputs {
            assert!(Wrap::UuidV5Namespaced.apply(input).is_some());
            assert!(Wrap::UuidV5With(ns).apply(input).is_some());
            assert!(Wrap::UuidV3Nil.apply(input).is_some());
        }
    }
}
