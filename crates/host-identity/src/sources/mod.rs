//! Built-in [`Source`] implementations.
//!
//! Every public struct in this module and its submodules implements
//! [`Source`] and can be composed into a [`crate::Resolver`] in any order.
//!
//! Platform-specific sources (e.g. [`WindowsMachineGuid`]) are available as
//! types on every platform so callers can reference them portably, but their
//! [`Source::probe`] only returns `Ok(Some(..))` on their native OS — on
//! other platforms they return `Ok(None)` and are silently skipped by the
//! resolver.

use crate::source::Source;

// Macros shared across the platform-specific stub modules. Always
// compiled; the individual stub modules that use them are cfg-gated.
mod stub_macros;

mod generic;
pub use generic::{EnvOverride, FileOverride, FnSource};

mod app_specific;
pub use app_specific::AppSpecific;

#[cfg(feature = "container")]
mod container;
#[cfg(feature = "container")]
pub use container::ContainerId;

#[cfg(feature = "container")]
mod lxc;
#[cfg(feature = "container")]
pub use lxc::LxcId;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(crate) use linux::in_container as linux_in_container;
#[cfg(target_os = "linux")]
pub use linux::{DbusMachineIdFile, DmiProductUuid, MachineIdFile};

// Stubs so callers can name these types on non-Linux platforms.
#[cfg(not(target_os = "linux"))]
mod linux_stubs;
#[cfg(not(target_os = "linux"))]
pub use linux_stubs::{DbusMachineIdFile, DmiProductUuid, MachineIdFile};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::IoPlatformUuid;
#[cfg(not(target_os = "macos"))]
mod macos_stubs;
#[cfg(not(target_os = "macos"))]
pub use macos_stubs::IoPlatformUuid;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::WindowsMachineGuid;
#[cfg(not(target_os = "windows"))]
mod windows_stubs;
#[cfg(not(target_os = "windows"))]
pub use windows_stubs::WindowsMachineGuid;

#[cfg(target_os = "freebsd")]
mod freebsd;
#[cfg(target_os = "freebsd")]
pub use freebsd::{FreeBsdHostIdFile, KenvSmbios};
#[cfg(not(target_os = "freebsd"))]
mod freebsd_stubs;
#[cfg(not(target_os = "freebsd"))]
pub use freebsd_stubs::{FreeBsdHostIdFile, KenvSmbios};

#[cfg(any(target_os = "openbsd", target_os = "netbsd"))]
mod bsd;
#[cfg(any(target_os = "openbsd", target_os = "netbsd"))]
pub use bsd::SysctlKernHostId;
#[cfg(not(any(target_os = "openbsd", target_os = "netbsd")))]
mod bsd_stubs;
#[cfg(not(any(target_os = "openbsd", target_os = "netbsd")))]
pub use bsd_stubs::SysctlKernHostId;

#[cfg(any(target_os = "illumos", target_os = "solaris"))]
mod illumos;
#[cfg(any(target_os = "illumos", target_os = "solaris"))]
pub use illumos::IllumosHostId;
#[cfg(not(any(target_os = "illumos", target_os = "solaris")))]
mod illumos_stubs;
#[cfg(not(any(target_os = "illumos", target_os = "solaris")))]
pub use illumos_stubs::IllumosHostId;

#[cfg(feature = "_transport")]
pub mod cloud;
#[cfg(feature = "_transport")]
pub use cloud::{CloudEndpoint, CloudMetadata};

#[cfg(feature = "aws")]
mod aws;
#[cfg(feature = "aws")]
pub use aws::AwsImds;

#[cfg(feature = "gcp")]
mod gcp;
#[cfg(feature = "gcp")]
pub use gcp::GcpMetadata;

#[cfg(feature = "azure")]
mod azure;
#[cfg(feature = "azure")]
pub use azure::AzureImds;

#[cfg(feature = "digitalocean")]
mod digitalocean;
#[cfg(feature = "digitalocean")]
pub use digitalocean::DigitalOceanMetadata;

#[cfg(feature = "hetzner")]
mod hetzner;
#[cfg(feature = "hetzner")]
pub use hetzner::HetznerMetadata;

#[cfg(feature = "oci")]
mod oci;
#[cfg(feature = "oci")]
pub use oci::OciMetadata;

#[cfg(feature = "k8s")]
mod kubernetes;
#[cfg(feature = "k8s")]
pub use kubernetes::{KubernetesDownwardApi, KubernetesPodUid, KubernetesServiceAccount};

mod util;
pub use util::{UNINITIALIZED_SENTINEL, normalize};

/// Default platform-appropriate source chain.
///
/// The caller is free to ignore this and build their own with
/// [`crate::Resolver::new`]. The default chain is:
///
/// 1. `EnvOverride` on `HOST_IDENTITY` — consistent across every platform.
/// 2. On Linux with the `container` feature, [`ContainerId`] — so containers
///    get their own identity rather than inheriting the host's.
/// 3. Platform-native sources in recommended order (see each OS's module).
#[must_use]
pub fn default_chain() -> Vec<Box<dyn Source>> {
    let mut chain: Vec<Box<dyn Source>> = Vec::new();
    chain.push(Box::new(EnvOverride::new("HOST_IDENTITY")));

    #[cfg(all(target_os = "linux", feature = "container"))]
    {
        chain.push(Box::new(ContainerId::default()));
        chain.push(Box::new(LxcId::default()));
    }

    #[cfg(target_os = "linux")]
    {
        chain.push(Box::new(MachineIdFile::default()));
        chain.push(Box::new(DbusMachineIdFile::default()));
        chain.push(Box::new(DmiProductUuid::default()));
    }
    #[cfg(target_os = "macos")]
    chain.push(Box::new(IoPlatformUuid::default()));
    #[cfg(target_os = "windows")]
    chain.push(Box::new(WindowsMachineGuid::default()));
    #[cfg(target_os = "freebsd")]
    {
        chain.push(Box::new(FreeBsdHostIdFile::default()));
        chain.push(Box::new(KenvSmbios::default()));
    }
    #[cfg(any(target_os = "openbsd", target_os = "netbsd"))]
    chain.push(Box::new(SysctlKernHostId::default()));
    #[cfg(any(target_os = "illumos", target_os = "solaris"))]
    chain.push(Box::new(IllumosHostId::default()));

    chain
}

/// Default chain for [`crate::Resolver::with_network_defaults`] — the
/// [`default_chain`] plus every enabled cloud and Kubernetes source,
/// ordered so that per-pod identity outranks per-container which outranks
/// per-instance which outranks per-host software state.
///
/// See [`crate::Resolver::with_network_defaults`] for the full ordering
/// contract and a rationale.
#[cfg(feature = "_transport")]
#[must_use]
pub fn network_default_chain<T>(transport: T) -> Vec<Box<dyn Source>>
where
    T: crate::transport::HttpTransport + Clone + 'static,
{
    let mut chain: Vec<Box<dyn Source>> = vec![Box::new(EnvOverride::new("HOST_IDENTITY"))];

    #[cfg(feature = "k8s")]
    chain.push(Box::new(KubernetesPodUid::default()));

    #[cfg(all(target_os = "linux", feature = "container"))]
    {
        chain.push(Box::new(ContainerId::default()));
        chain.push(Box::new(LxcId::default()));
    }

    #[cfg(feature = "aws")]
    chain.push(Box::new(AwsImds::new(transport.clone())));
    #[cfg(feature = "gcp")]
    chain.push(Box::new(GcpMetadata::new(transport.clone())));
    #[cfg(feature = "azure")]
    chain.push(Box::new(AzureImds::new(transport.clone())));
    #[cfg(feature = "digitalocean")]
    chain.push(Box::new(DigitalOceanMetadata::new(transport.clone())));
    #[cfg(feature = "hetzner")]
    chain.push(Box::new(HetznerMetadata::new(transport.clone())));
    #[cfg(feature = "oci")]
    chain.push(Box::new(OciMetadata::new(transport.clone())));

    // The `transport` value is consumed by each cloud-source branch above
    // via `.clone()`. When no cloud feature is on (but `_transport` is,
    // which is how this function is gated) the parameter would be unused
    // — but `_transport` is only ever enabled *by* a cloud feature, so in
    // every real build at least one branch fires. Drop explicitly to keep
    // the intent obvious.
    drop(transport);

    #[cfg(target_os = "linux")]
    {
        chain.push(Box::new(MachineIdFile::default()));
        chain.push(Box::new(DbusMachineIdFile::default()));
        chain.push(Box::new(DmiProductUuid::default()));
    }
    #[cfg(target_os = "macos")]
    chain.push(Box::new(IoPlatformUuid::default()));
    #[cfg(target_os = "windows")]
    chain.push(Box::new(WindowsMachineGuid::default()));
    #[cfg(target_os = "freebsd")]
    {
        chain.push(Box::new(FreeBsdHostIdFile::default()));
        chain.push(Box::new(KenvSmbios::default()));
    }
    #[cfg(any(target_os = "openbsd", target_os = "netbsd"))]
    chain.push(Box::new(SysctlKernHostId::default()));
    #[cfg(any(target_os = "illumos", target_os = "solaris"))]
    chain.push(Box::new(IllumosHostId::default()));

    #[cfg(feature = "k8s")]
    chain.push(Box::new(KubernetesServiceAccount::default()));

    chain
}
