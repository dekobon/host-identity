//! The [`Source`] trait and associated types.

use std::fmt;

use crate::error::Error;

/// A single identity source.
///
/// Sources are composed into a [`crate::Resolver`]. The resolver walks them
/// in order and uses the first one that yields a [`Probe`]; sources that
/// have nothing to contribute (file missing, feature disabled, wrong
/// platform) return `Ok(None)` and are skipped.
///
/// Implementations must be inexpensive to construct — the resolver may
/// instantiate them in advance — but probing may perform I/O or spawn a
/// subprocess.
pub trait Source: fmt::Debug + Send + Sync {
    /// Provenance label for this source. Shown in error messages and the
    /// resolved [`crate::HostId`].
    fn kind(&self) -> SourceKind;

    /// Attempt to produce a raw identifier.
    ///
    /// - `Ok(Some(probe))` — a usable identifier was found
    /// - `Ok(None)` — this source had nothing to offer; the resolver
    ///   continues to the next one
    /// - `Err(_)` — a hard failure the caller should know about (permission
    ///   denied, malformed registry entry, sentinel value like
    ///   `uninitialized`)
    fn probe(&self) -> Result<Option<Probe>, Error>;
}

impl<T: Source + ?Sized> Source for Box<T> {
    fn kind(&self) -> SourceKind {
        (**self).kind()
    }
    fn probe(&self) -> Result<Option<Probe>, Error> {
        (**self).probe()
    }
}

/// A raw identifier returned by a [`Source`], before UUID wrapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    kind: SourceKind,
    value: String,
}

impl Probe {
    /// Construct a probe. The caller is responsible for trimming and
    /// sentinel-rejection; see [`crate::sources::normalize`].
    #[must_use]
    pub fn new(kind: SourceKind, value: impl Into<String>) -> Self {
        Self {
            kind,
            value: value.into(),
        }
    }

    /// Which source produced this value.
    #[must_use]
    pub fn kind(&self) -> SourceKind {
        self.kind
    }

    /// The raw string value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    pub(crate) fn into_parts(self) -> (SourceKind, String) {
        (self.kind, self.value)
    }
}

/// Short, stable label identifying a source.
///
/// Covers every built-in source plus a [`SourceKind::Custom`] variant for
/// consumer-defined sources. Displayed in error messages, logs, and on the
/// resolved [`crate::HostId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SourceKind {
    /// Environment-variable override.
    EnvOverride,
    /// File-path override.
    FileOverride,
    /// Container runtime ID extracted from `/proc/self/mountinfo` (Linux).
    Container,
    /// LXC/LXD container name from `/proc/self/cgroup` or
    /// `/proc/self/mountinfo`, salted with `/etc/machine-id` (Linux).
    Lxc,
    /// `/etc/machine-id` (Linux).
    MachineId,
    /// `/var/lib/dbus/machine-id` (Linux).
    DbusMachineId,
    /// `/sys/class/dmi/id/product_uuid` — SMBIOS system UUID (Linux).
    Dmi,
    /// `IOPlatformUUID` from `IOPlatformExpertDevice` (macOS).
    IoPlatformUuid,
    /// `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid` (Windows).
    WindowsMachineGuid,
    /// `/etc/hostid` (FreeBSD).
    FreeBsdHostId,
    /// `kenv smbios.system.uuid` (FreeBSD SMBIOS).
    KenvSmbios,
    /// `sysctl kern.hostid` (NetBSD, OpenBSD).
    BsdKernHostId,
    /// `hostid(1)` (illumos, Solaris).
    IllumosHostId,
    /// AWS EC2 instance ID via `IMDSv2`.
    AwsImds,
    /// GCP Compute Engine numeric instance ID via the metadata server.
    GcpMetadata,
    /// Azure VM UUID via the Azure Instance Metadata Service.
    AzureImds,
    /// `DigitalOcean` Droplet numeric ID.
    DigitalOceanMetadata,
    /// Hetzner Cloud numeric server ID.
    HetznerMetadata,
    /// Oracle Cloud Infrastructure instance OCID.
    OciMetadata,
    /// Kubernetes pod UID derived from `/proc/self/mountinfo`.
    KubernetesPodUid,
    /// Kubernetes service-account namespace.
    KubernetesServiceAccount,
    /// Kubernetes downward-API projected file.
    KubernetesDownwardApi,
    /// Caller-supplied source; the payload is a short label for logs.
    ///
    /// The label appears verbatim in `Display` and
    /// [`SourceKind::as_str`] output — a `Custom("machine-id")` renders
    /// identically to the built-in [`SourceKind::MachineId`]. Callers
    /// should pick labels that don't collide with the built-in
    /// identifiers listed in [`crate::ids::source_ids`] so operators
    /// reading logs can tell which source a probe came from.
    Custom(&'static str),
}

/// Generate `as_str` and `from_id` from a single variant↔string table.
/// Keeps the two methods in lockstep — adding a variant means adding
/// one row.
macro_rules! source_kind_ids {
    ( $( $variant:ident => $id:literal , $desc:literal );* $(;)? ) => {
        impl SourceKind {
            /// Construct a [`SourceKind::Custom`] from a static string label.
            #[must_use]
            pub const fn custom(label: &'static str) -> Self {
                Self::Custom(label)
            }

            /// Short, stable, lowercase name suitable for logs and telemetry.
            #[must_use]
            pub fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $id, )*
                    Self::Custom(label) => label,
                }
            }

            /// Inverse of [`SourceKind::as_str`] for the built-in identifiers.
            ///
            /// Returns `Some(kind)` when `id` matches one of the stable
            /// strings returned by `as_str` for a non-`Custom` variant,
            /// `None` otherwise. `SourceKind::Custom` intentionally never
            /// round-trips through this — a runtime string cannot safely
            /// become a `&'static str`.
            #[must_use]
            pub fn from_id(id: &str) -> Option<Self> {
                match id {
                    $( $id => Some(Self::$variant), )*
                    _ => None,
                }
            }

            /// One-line plain-text description of where this source reads
            /// its identifier from. `Custom` returns an empty string —
            /// callers supply their own labels.
            #[must_use]
            pub fn describe(self) -> &'static str {
                match self {
                    $( Self::$variant => $desc, )*
                    Self::Custom(_) => "",
                }
            }
        }
    };
}

source_kind_ids! {
    EnvOverride              => "env-override",              "environment-variable override (HOST_IDENTITY by default)";
    FileOverride             => "file-override",             "caller-supplied file containing a host identifier";
    Container                => "container",                 "container runtime ID from /proc/self/mountinfo (Linux)";
    Lxc                      => "lxc",                       "LXC/LXD container name from /proc/self/cgroup or /proc/self/mountinfo, salted with /etc/machine-id (Linux)";
    MachineId                => "machine-id",                "/etc/machine-id (Linux)";
    DbusMachineId            => "dbus-machine-id",           "/var/lib/dbus/machine-id (Linux)";
    Dmi                      => "dmi",                       "/sys/class/dmi/id/product_uuid — SMBIOS system UUID (Linux)";
    IoPlatformUuid           => "io-platform-uuid",          "IOPlatformUUID from IOPlatformExpertDevice (macOS)";
    WindowsMachineGuid       => "windows-machine-guid",      "HKLM\\SOFTWARE\\Microsoft\\Cryptography\\MachineGuid (Windows)";
    FreeBsdHostId            => "freebsd-hostid",            "/etc/hostid (FreeBSD)";
    KenvSmbios               => "kenv-smbios",               "kenv smbios.system.uuid — SMBIOS system UUID (FreeBSD)";
    BsdKernHostId            => "bsd-kern-hostid",           "sysctl kern.hostid (NetBSD, OpenBSD)";
    IllumosHostId            => "illumos-hostid",            "hostid(1) (illumos, Solaris)";
    AwsImds                  => "aws-imds",                  "AWS EC2 instance ID via IMDSv2";
    GcpMetadata              => "gcp-metadata",              "GCP Compute Engine numeric instance ID via the metadata server";
    AzureImds                => "azure-imds",                "Azure VM UUID via the Azure Instance Metadata Service";
    DigitalOceanMetadata     => "digital-ocean-metadata",    "DigitalOcean Droplet numeric ID via the metadata service";
    HetznerMetadata          => "hetzner-metadata",          "Hetzner Cloud numeric server ID via the metadata service";
    OciMetadata              => "oci-metadata",              "Oracle Cloud Infrastructure instance OCID via the metadata service";
    KubernetesPodUid         => "kubernetes-pod-uid",        "Kubernetes pod UID derived from /proc/self/mountinfo";
    KubernetesServiceAccount => "kubernetes-service-account","Kubernetes service-account namespace from the projected token volume";
    KubernetesDownwardApi    => "kubernetes-downward-api",   "caller-supplied Kubernetes downward-API projected file";
}

impl fmt::Display for SourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
