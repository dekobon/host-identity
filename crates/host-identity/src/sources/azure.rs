//! Azure IMDS instance-metadata identity source.
//!
//! `GET http://169.254.169.254/metadata/instance/compute/vmId` with
//! `api-version` and `format=text` query params, authenticated by the
//! `Metadata: true` header. Response is the plaintext VM UUID.
//!
//! Authoritative reference:
//! [Azure Instance Metadata Service for virtual machines](https://learn.microsoft.com/en-us/azure/virtual-machines/instance-metadata-service)
//! — mandates the `Metadata: true` header, an `api-version` query parameter,
//! and documents the `/metadata/instance/compute/vmId` leaf. This source
//! pins `api-version=2021-02-01`, a stable version; bump only after
//! validating that newer versions still return the same `vmId` semantics.

use crate::source::SourceKind;
use crate::sources::cloud::{CloudEndpoint, CloudMetadata};

/// Azure VM UUID via the Azure Instance Metadata Service.
pub type AzureImds<T> = CloudMetadata<AzureEndpoint, T>;

/// Endpoint descriptor for [`AzureImds`].
pub struct AzureEndpoint;

impl CloudEndpoint for AzureEndpoint {
    const DEBUG_NAME: &'static str = "AzureImds";
    const DEFAULT_BASE_URL: &'static str = "http://169.254.169.254";
    const PATH: &'static str = "/metadata/instance/compute/vmId?api-version=2021-02-01&format=text";
    const KIND: SourceKind = SourceKind::AzureImds;

    fn headers() -> &'static [(&'static str, &'static str)] {
        &[("Metadata", "true")]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Source;
    use crate::sources::cloud::test_support::{StubTransport, ok, status};

    const VM_ID: &str = "02aab8a4-74ef-476e-8182-f6d2ba4166a6";

    #[test]
    fn happy_path_returns_vm_id() {
        let source = AzureImds::new(StubTransport::new(vec![ok(VM_ID)]));
        let probe = source.probe().unwrap().expect("should produce a probe");
        assert_eq!(probe.kind(), SourceKind::AzureImds);
        assert_eq!(probe.value(), VM_ID);
    }

    #[test]
    fn sends_metadata_header_and_api_version() {
        let (stub, transport) = StubTransport::shared(vec![ok(VM_ID)]);
        let source = AzureImds::with_base_url(transport, "http://imds.test");
        assert!(source.probe().unwrap().is_some());

        let requests = stub.requests();
        assert_eq!(requests.len(), 1);
        let (method, uri, headers) = &requests[0];
        assert_eq!(method, http::Method::GET);
        assert_eq!(
            uri,
            "http://imds.test/metadata/instance/compute/vmId?api-version=2021-02-01&format=text"
        );
        assert_eq!(headers.get("Metadata").unwrap(), "true");
    }

    #[test]
    fn non_2xx_returns_none() {
        let source = AzureImds::new(StubTransport::new(vec![status(400)]));
        assert!(source.probe().unwrap().is_none());
    }
}
