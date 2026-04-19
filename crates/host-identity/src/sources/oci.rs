//! Oracle Cloud Infrastructure (OCI) instance-metadata identity source.
//!
//! `GET http://169.254.169.254/opc/v2/instance/id` with
//! `Authorization: Bearer Oracle`. The v2 endpoint returns the OCID as
//! plaintext. The Authorization header is mandatory on v2 to prevent
//! cross-container metadata theft.
//!
//! Authoritative reference:
//! [OCI: Getting instance metadata](https://docs.oracle.com/en-us/iaas/Content/Compute/Tasks/gettingmetadata.htm)
//! — documents the `/opc/v2/` prefix, the mandatory
//! `Authorization: Bearer Oracle` header, the rejection of forwarded
//! headers, and the `id` field (the OCID) under `/opc/v2/instance/`.

use crate::source::SourceKind;
use crate::sources::cloud::{CloudEndpoint, CloudMetadata};

/// OCI instance OCID via the OPC metadata v2 endpoint.
pub type OciMetadata<T> = CloudMetadata<OciEndpoint, T>;

/// Endpoint descriptor for [`OciMetadata`].
pub struct OciEndpoint;

impl CloudEndpoint for OciEndpoint {
    const DEBUG_NAME: &'static str = "OciMetadata";
    const DEFAULT_BASE_URL: &'static str = "http://169.254.169.254";
    const PATH: &'static str = "/opc/v2/instance/id";
    const KIND: SourceKind = SourceKind::OciMetadata;

    fn headers() -> &'static [(&'static str, &'static str)] {
        &[("Authorization", "Bearer Oracle")]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Source;
    use crate::sources::cloud::test_support::{StubTransport, ok, status};

    const OCID: &str =
        "ocid1.instance.oc1.phx.anyhqljr6rrnepaccncpufxzlsycspzkebcsbn2fsvzbquqarjgxo3yzxtnq";

    #[test]
    fn happy_path_returns_ocid() {
        let source = OciMetadata::new(StubTransport::new(vec![ok(OCID)]));
        let probe = source.probe().unwrap().expect("should produce a probe");
        assert_eq!(probe.kind(), SourceKind::OciMetadata);
        assert_eq!(probe.value(), OCID);
    }

    #[test]
    fn sends_bearer_authorization_header() {
        let (stub, transport) = StubTransport::shared(vec![ok("ocid1.x")]);
        let source = OciMetadata::with_base_url(transport, "http://opc.test");
        assert!(source.probe().unwrap().is_some());

        let requests = stub.requests();
        assert_eq!(requests.len(), 1);
        let (method, uri, headers) = &requests[0];
        assert_eq!(method, http::Method::GET);
        assert_eq!(uri, "http://opc.test/opc/v2/instance/id");
        assert_eq!(headers.get("Authorization").unwrap(), "Bearer Oracle");
    }

    #[test]
    fn non_2xx_returns_none() {
        // v2 returns 401 when the Authorization header is missing; any
        // non-2xx is treated the same by the source.
        let source = OciMetadata::new(StubTransport::new(vec![status(401)]));
        assert!(source.probe().unwrap().is_none());
    }
}
