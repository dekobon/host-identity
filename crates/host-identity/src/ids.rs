//! Identifier-based chain construction.
//!
//! Build a [`crate::Resolver`] from a list of string identifiers,
//! the way an operator would specify sources in a config file. Complements
//! the typed builder API: the typed constructors (`MachineIdFile::default`,
//! `AwsImds::new(t)`, …) stay available and take precedence when you need
//! non-default parameters; the identifier API covers the common
//! "reasonable defaults, list in config" workflow.
//!
//! # Example
//!
//! ```
//! # use host_identity::ids::{resolver_from_ids, source_ids};
//! let resolver = resolver_from_ids([
//!     source_ids::ENV_OVERRIDE,
//!     source_ids::MACHINE_ID,
//!     source_ids::DMI,
//! ]).unwrap();
//! ```
//!
//! # Identifiers
//!
//! The identifier for every built-in source is the string returned by
//! [`SourceKind::as_str`]. Stable constants live in [`source_ids`] for
//! compile-time typo catching.
//!
//! Two identifiers are recognised by [`SourceKind::from_id`] but cannot
//! be built by [`resolver_from_ids`] because they need a caller-supplied
//! path — `"file-override"` and `"kubernetes-downward-api"`. Passing
//! either returns [`UnknownSourceError::RequiresPath`]; construct them
//! manually with their typed constructors and `.push(...)` them onto
//! the returned resolver.
//!
//! Cloud identifiers (`"aws-imds"`, `"gcp-metadata"`, …) need an HTTP
//! transport — [`resolver_from_ids`] rejects them with
//! [`UnknownSourceError::RequiresTransport`]; use
//! [`resolver_from_ids_with_transport`] instead.

use crate::source::{Source, SourceKind};
use crate::{Resolver, sources};

/// Stable identifier strings for every built-in source. Use these over
/// raw string literals to catch typos at compile time.
pub mod source_ids {
    /// `"env-override"` — [`crate::sources::EnvOverride`] with the default
    /// `HOST_IDENTITY` variable name.
    pub const ENV_OVERRIDE: &str = "env-override";
    /// `"file-override"` — [`crate::sources::FileOverride`]. Not
    /// default-constructible; resolves to
    /// [`super::UnknownSourceError::RequiresPath`] in identifier-based
    /// builders.
    pub const FILE_OVERRIDE: &str = "file-override";
    /// `"container"` — [`crate::sources::ContainerId`] (feature `container`).
    pub const CONTAINER: &str = "container";
    /// `"lxc"` — [`crate::sources::LxcId`] (feature `container`).
    pub const LXC: &str = "lxc";
    /// `"machine-id"` — [`crate::sources::MachineIdFile`].
    pub const MACHINE_ID: &str = "machine-id";
    /// `"dbus-machine-id"` — [`crate::sources::DbusMachineIdFile`].
    pub const DBUS_MACHINE_ID: &str = "dbus-machine-id";
    /// `"dmi"` — [`crate::sources::DmiProductUuid`].
    pub const DMI: &str = "dmi";
    /// `"linux-hostid"` — [`crate::sources::LinuxHostIdFile`]. Opt-in;
    /// not part of either default chain.
    pub const LINUX_HOSTID: &str = "linux-hostid";
    /// `"io-platform-uuid"` — [`crate::sources::IoPlatformUuid`].
    pub const IO_PLATFORM_UUID: &str = "io-platform-uuid";
    /// `"windows-machine-guid"` — [`crate::sources::WindowsMachineGuid`].
    pub const WINDOWS_MACHINE_GUID: &str = "windows-machine-guid";
    /// `"freebsd-hostid"` — [`crate::sources::FreeBsdHostIdFile`].
    pub const FREEBSD_HOSTID: &str = "freebsd-hostid";
    /// `"kenv-smbios"` — [`crate::sources::KenvSmbios`].
    pub const KENV_SMBIOS: &str = "kenv-smbios";
    /// `"bsd-kern-hostid"` — [`crate::sources::SysctlKernHostId`].
    pub const BSD_KERN_HOSTID: &str = "bsd-kern-hostid";
    /// `"illumos-hostid"` — [`crate::sources::IllumosHostId`].
    pub const ILLUMOS_HOSTID: &str = "illumos-hostid";
    /// `"aws-imds"` — [`crate::sources::AwsImds`]. Requires transport.
    pub const AWS_IMDS: &str = "aws-imds";
    /// `"gcp-metadata"` — [`crate::sources::GcpMetadata`]. Requires transport.
    pub const GCP_METADATA: &str = "gcp-metadata";
    /// `"azure-imds"` — [`crate::sources::AzureImds`]. Requires transport.
    pub const AZURE_IMDS: &str = "azure-imds";
    /// `"digital-ocean-metadata"` — [`crate::sources::DigitalOceanMetadata`].
    /// Requires transport.
    pub const DIGITAL_OCEAN_METADATA: &str = "digital-ocean-metadata";
    /// `"hetzner-metadata"` — [`crate::sources::HetznerMetadata`]. Requires
    /// transport.
    pub const HETZNER_METADATA: &str = "hetzner-metadata";
    /// `"oci-metadata"` — [`crate::sources::OciMetadata`]. Requires transport.
    pub const OCI_METADATA: &str = "oci-metadata";
    /// `"kubernetes-pod-uid"` — [`crate::sources::KubernetesPodUid`].
    pub const KUBERNETES_POD_UID: &str = "kubernetes-pod-uid";
    /// `"kubernetes-service-account"` — [`crate::sources::KubernetesServiceAccount`].
    pub const KUBERNETES_SERVICE_ACCOUNT: &str = "kubernetes-service-account";
    /// `"kubernetes-downward-api"` — [`crate::sources::KubernetesDownwardApi`].
    /// Not default-constructible; needs a path.
    pub const KUBERNETES_DOWNWARD_API: &str = "kubernetes-downward-api";
}

/// Reasons an identifier-based chain could not be built.
#[derive(Debug, thiserror::Error)]
pub enum UnknownSourceError {
    /// The identifier didn't match any built-in source.
    #[error("unknown source identifier: `{0}`")]
    Unknown(String),
    /// The identifier names a source that requires a caller-supplied path
    /// (e.g. `file-override`, `kubernetes-downward-api`). Build it
    /// manually with the typed constructor and chain via `.push(...)`.
    #[error(
        "source `{0}` requires a caller-supplied path; construct it with its typed constructor and push it manually"
    )]
    RequiresPath(&'static str),
    /// The identifier names a cloud source that needs an HTTP transport;
    /// use [`resolver_from_ids_with_transport`].
    #[error("source `{0}` requires an HTTP transport; use resolver_from_ids_with_transport")]
    RequiresTransport(&'static str),
    /// The identifier is valid but its crate feature is not enabled in
    /// this build.
    #[error("source `{0}` is not available — the `{1}` feature is not enabled")]
    FeatureDisabled(&'static str, &'static str),
}

/// Build a [`Resolver`] from a list of source identifiers. Local sources
/// only — cloud identifiers return [`UnknownSourceError::RequiresTransport`].
///
/// The returned resolver has the identifiers' sources in the order they
/// were supplied. Call `.push(...)` / `.prepend(...)` on it to add
/// typed-constructor sources (e.g. `FileOverride::new(path)`) that
/// can't be built from an identifier alone.
///
/// # Errors
///
/// Returns [`UnknownSourceError`] on the first unrecognised, path-requiring,
/// transport-requiring, or feature-disabled identifier.
pub fn resolver_from_ids<S, I>(ids: I) -> Result<Resolver, UnknownSourceError>
where
    S: AsRef<str>,
    I: IntoIterator<Item = S>,
{
    let mut resolver = Resolver::new();
    for id in ids {
        let source = local_source_from_id(id.as_ref())?;
        resolver = resolver.push_boxed(source);
    }
    Ok(resolver)
}

/// Build a [`Resolver`] from a list of source identifiers, with an HTTP
/// transport available for cloud sources.
///
/// Accepts the same identifiers as [`resolver_from_ids`] plus every
/// enabled cloud source (`aws-imds`, `gcp-metadata`, `azure-imds`,
/// `digital-ocean-metadata`, `hetzner-metadata`, `oci-metadata`).
///
/// # Errors
///
/// As [`resolver_from_ids`], minus [`UnknownSourceError::RequiresTransport`]
/// (which can't occur here).
#[cfg(feature = "_transport")]
#[allow(
    clippy::needless_pass_by_value,
    reason = "by-value transport matches `resolve_with_transport` and `Resolver::with_network_defaults`; the final clone drops the original"
)]
pub fn resolver_from_ids_with_transport<S, I, T>(
    ids: I,
    transport: T,
) -> Result<Resolver, UnknownSourceError>
where
    S: AsRef<str>,
    I: IntoIterator<Item = S>,
    T: crate::transport::HttpTransport + Clone + 'static,
{
    let mut resolver = Resolver::new();
    for id in ids {
        let source = source_from_id_with_transport(id.as_ref(), transport.clone())?;
        resolver = resolver.push_boxed(source);
    }
    Ok(resolver)
}

/// Expand to `Ok(Box::new(ctor))` when the feature is on, or to a
/// `FeatureDisabled` error when it isn't. Used by both source lookups
/// below to keep the feature-gated arms to one line each.
macro_rules! feature_ctor {
    ($feature:literal, $id:literal, $ctor:expr) => {{
        #[cfg(feature = $feature)]
        {
            Ok(Box::new($ctor))
        }
        #[cfg(not(feature = $feature))]
        {
            Err(UnknownSourceError::FeatureDisabled($id, $feature))
        }
    }};
}

fn local_source_from_id(id: &str) -> Result<Box<dyn Source>, UnknownSourceError> {
    let kind = SourceKind::from_id(id).ok_or_else(|| UnknownSourceError::Unknown(id.to_owned()))?;
    non_constructible_local(kind)
        .or_else(|| feature_gated_local(kind))
        .unwrap_or_else(|| Ok(plain_local(kind)))
}

// Variants whose construction either needs no work (`EnvOverride`),
// needs a caller-supplied path (`FileOverride`, `KubernetesDownwardApi`),
// or needs an HTTP transport (the cloud kinds).
fn non_constructible_local(
    kind: SourceKind,
) -> Option<Result<Box<dyn Source>, UnknownSourceError>> {
    match kind {
        SourceKind::EnvOverride => Some(Ok(Box::new(sources::EnvOverride::new("HOST_IDENTITY")))),
        SourceKind::FileOverride => Some(Err(UnknownSourceError::RequiresPath("file-override"))),
        SourceKind::KubernetesDownwardApi => Some(Err(UnknownSourceError::RequiresPath(
            "kubernetes-downward-api",
        ))),
        SourceKind::AwsImds
        | SourceKind::GcpMetadata
        | SourceKind::AzureImds
        | SourceKind::DigitalOceanMetadata
        | SourceKind::HetznerMetadata
        | SourceKind::OciMetadata => {
            Some(Err(UnknownSourceError::RequiresTransport(kind.as_str())))
        }
        _ => None,
    }
}

// Variants whose constructor is gated behind a crate feature.
fn feature_gated_local(kind: SourceKind) -> Option<Result<Box<dyn Source>, UnknownSourceError>> {
    Some(match kind {
        SourceKind::Container => {
            feature_ctor!("container", "container", sources::ContainerId::default())
        }
        SourceKind::Lxc => feature_ctor!("container", "lxc", sources::LxcId::default()),
        SourceKind::KubernetesPodUid => feature_ctor!(
            "k8s",
            "kubernetes-pod-uid",
            sources::KubernetesPodUid::default()
        ),
        SourceKind::KubernetesServiceAccount => feature_ctor!(
            "k8s",
            "kubernetes-service-account",
            sources::KubernetesServiceAccount::default()
        ),
        _ => return None,
    })
}

// Always-available platform-typed sources. Dispatched by platform
// family so no single helper matches every variant at once.
fn plain_local(kind: SourceKind) -> Box<dyn Source> {
    linux_family_source(kind)
        .or_else(|| native_non_linux_source(kind))
        .unwrap_or_else(|| unreachable!("plain_local reached with unhandled kind: {kind:?}"))
}

fn linux_family_source(kind: SourceKind) -> Option<Box<dyn Source>> {
    Some(match kind {
        SourceKind::MachineId => Box::new(sources::MachineIdFile::default()),
        SourceKind::DbusMachineId => Box::new(sources::DbusMachineIdFile::default()),
        SourceKind::Dmi => Box::new(sources::DmiProductUuid::default()),
        SourceKind::LinuxHostId => Box::new(sources::LinuxHostIdFile::default()),
        _ => return None,
    })
}

fn native_non_linux_source(kind: SourceKind) -> Option<Box<dyn Source>> {
    Some(match kind {
        SourceKind::IoPlatformUuid => Box::new(sources::IoPlatformUuid::default()),
        SourceKind::WindowsMachineGuid => Box::new(sources::WindowsMachineGuid::default()),
        SourceKind::FreeBsdHostId => Box::new(sources::FreeBsdHostIdFile::default()),
        SourceKind::KenvSmbios => Box::new(sources::KenvSmbios::default()),
        SourceKind::BsdKernHostId => Box::new(sources::SysctlKernHostId::default()),
        SourceKind::IllumosHostId => Box::new(sources::IllumosHostId::default()),
        _ => return None,
    })
}

#[cfg(feature = "_transport")]
fn source_from_id_with_transport<T>(
    id: &str,
    transport: T,
) -> Result<Box<dyn Source>, UnknownSourceError>
where
    T: crate::transport::HttpTransport + Clone + 'static,
{
    let kind = SourceKind::from_id(id).ok_or_else(|| UnknownSourceError::Unknown(id.to_owned()))?;
    match kind {
        SourceKind::AwsImds => feature_ctor!("aws", "aws-imds", sources::AwsImds::new(transport)),
        SourceKind::GcpMetadata => {
            feature_ctor!("gcp", "gcp-metadata", sources::GcpMetadata::new(transport))
        }
        SourceKind::AzureImds => {
            feature_ctor!("azure", "azure-imds", sources::AzureImds::new(transport))
        }
        SourceKind::DigitalOceanMetadata => feature_ctor!(
            "digitalocean",
            "digital-ocean-metadata",
            sources::DigitalOceanMetadata::new(transport)
        ),
        SourceKind::HetznerMetadata => feature_ctor!(
            "hetzner",
            "hetzner-metadata",
            sources::HetznerMetadata::new(transport)
        ),
        SourceKind::OciMetadata => {
            feature_ctor!("oci", "oci-metadata", sources::OciMetadata::new(transport))
        }
        _ => {
            // Drop the cloned transport explicitly — the fallback path
            // doesn't need it, and holding onto a clone until the end of
            // scope would defer closing any transport-held resources
            // (sockets, client handles) for no reason.
            drop(transport);
            local_source_from_id(id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_kind_from_id_round_trips_every_builtin() {
        for kind in [
            SourceKind::EnvOverride,
            SourceKind::FileOverride,
            SourceKind::Container,
            SourceKind::Lxc,
            SourceKind::MachineId,
            SourceKind::DbusMachineId,
            SourceKind::Dmi,
            SourceKind::LinuxHostId,
            SourceKind::IoPlatformUuid,
            SourceKind::WindowsMachineGuid,
            SourceKind::FreeBsdHostId,
            SourceKind::KenvSmbios,
            SourceKind::BsdKernHostId,
            SourceKind::IllumosHostId,
            SourceKind::AwsImds,
            SourceKind::GcpMetadata,
            SourceKind::AzureImds,
            SourceKind::DigitalOceanMetadata,
            SourceKind::HetznerMetadata,
            SourceKind::OciMetadata,
            SourceKind::KubernetesPodUid,
            SourceKind::KubernetesServiceAccount,
            SourceKind::KubernetesDownwardApi,
        ] {
            assert_eq!(SourceKind::from_id(kind.as_str()), Some(kind));
        }
    }

    // Guards against a future `SourceKind` variant being added without
    // wiring it into the three-stage dispatch inside `local_source_from_id`.
    // The exhaustive match that used to catch this at compile time was
    // split into family-scoped helpers with `_ => return None` arms; this
    // test closes that gap by ensuring every built-in identifier either
    // constructs successfully or surfaces a documented `UnknownSourceError`
    // — never a runtime `unreachable!` panic.
    #[test]
    fn local_source_from_id_handles_every_builtin_identifier() {
        for id in [
            source_ids::ENV_OVERRIDE,
            source_ids::FILE_OVERRIDE,
            source_ids::CONTAINER,
            source_ids::LXC,
            source_ids::MACHINE_ID,
            source_ids::DBUS_MACHINE_ID,
            source_ids::DMI,
            source_ids::LINUX_HOSTID,
            source_ids::IO_PLATFORM_UUID,
            source_ids::WINDOWS_MACHINE_GUID,
            source_ids::FREEBSD_HOSTID,
            source_ids::KENV_SMBIOS,
            source_ids::BSD_KERN_HOSTID,
            source_ids::ILLUMOS_HOSTID,
            source_ids::AWS_IMDS,
            source_ids::GCP_METADATA,
            source_ids::AZURE_IMDS,
            source_ids::DIGITAL_OCEAN_METADATA,
            source_ids::HETZNER_METADATA,
            source_ids::OCI_METADATA,
            source_ids::KUBERNETES_POD_UID,
            source_ids::KUBERNETES_SERVICE_ACCOUNT,
            source_ids::KUBERNETES_DOWNWARD_API,
        ] {
            match local_source_from_id(id) {
                Ok(_)
                | Err(
                    UnknownSourceError::RequiresPath(_)
                    | UnknownSourceError::RequiresTransport(_)
                    | UnknownSourceError::FeatureDisabled(_, _),
                ) => {}
                Err(UnknownSourceError::Unknown(got)) => {
                    panic!("identifier `{id}` was reported as unknown (got `{got}`)");
                }
            }
        }
    }

    #[test]
    fn source_kind_from_id_rejects_unknown() {
        assert_eq!(SourceKind::from_id("not-a-real-source"), None);
        assert_eq!(SourceKind::from_id(""), None);
        // Custom variants intentionally don't round-trip.
        assert_eq!(SourceKind::from_id("my-custom-source"), None);
    }

    #[test]
    fn resolver_from_ids_builds_chain_in_order() {
        let resolver =
            resolver_from_ids([source_ids::ENV_OVERRIDE, source_ids::MACHINE_ID]).unwrap();
        assert_eq!(
            resolver.source_kinds(),
            vec![SourceKind::EnvOverride, SourceKind::MachineId]
        );
    }

    #[test]
    fn resolver_from_ids_rejects_unknown_identifier() {
        match resolver_from_ids(["machine-id", "not-real"]).unwrap_err() {
            UnknownSourceError::Unknown(s) => assert_eq!(s, "not-real"),
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn resolver_from_ids_rejects_path_requiring_sources() {
        match resolver_from_ids([source_ids::FILE_OVERRIDE]).unwrap_err() {
            UnknownSourceError::RequiresPath(id) => assert_eq!(id, "file-override"),
            other => panic!("expected RequiresPath, got {other:?}"),
        }
        #[cfg(feature = "k8s")]
        match resolver_from_ids([source_ids::KUBERNETES_DOWNWARD_API]).unwrap_err() {
            UnknownSourceError::RequiresPath(id) => {
                assert_eq!(id, "kubernetes-downward-api");
            }
            other => panic!("expected RequiresPath, got {other:?}"),
        }
    }

    #[cfg(feature = "aws")]
    #[test]
    fn resolver_from_ids_rejects_cloud_ids_without_transport() {
        match resolver_from_ids([source_ids::AWS_IMDS]).unwrap_err() {
            UnknownSourceError::RequiresTransport(id) => assert_eq!(id, "aws-imds"),
            other => panic!("expected RequiresTransport, got {other:?}"),
        }
    }

    #[cfg(feature = "aws")]
    #[test]
    fn resolver_from_ids_with_transport_accepts_cloud_ids() {
        use crate::transport::HttpTransport;
        use std::convert::Infallible;

        #[derive(Clone)]
        struct NoopTransport;
        impl HttpTransport for NoopTransport {
            type Error = Infallible;
            fn send(
                &self,
                _req: http::Request<Vec<u8>>,
            ) -> Result<http::Response<Vec<u8>>, Self::Error> {
                Ok(http::Response::builder()
                    .status(404)
                    .body(Vec::new())
                    .unwrap())
            }
        }

        let resolver = resolver_from_ids_with_transport(
            [
                source_ids::ENV_OVERRIDE,
                source_ids::AWS_IMDS,
                source_ids::MACHINE_ID,
            ],
            NoopTransport,
        )
        .unwrap();
        assert_eq!(
            resolver.source_kinds(),
            vec![
                SourceKind::EnvOverride,
                SourceKind::AwsImds,
                SourceKind::MachineId
            ],
        );
    }

    #[cfg(not(feature = "k8s"))]
    #[test]
    fn resolver_from_ids_reports_feature_disabled() {
        match resolver_from_ids([source_ids::KUBERNETES_POD_UID]).unwrap_err() {
            UnknownSourceError::FeatureDisabled(id, feat) => {
                assert_eq!(id, "kubernetes-pod-uid");
                assert_eq!(feat, "k8s");
            }
            other => panic!("expected FeatureDisabled, got {other:?}"),
        }
    }
}
