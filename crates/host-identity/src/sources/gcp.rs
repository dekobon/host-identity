//! GCP Compute Engine instance-metadata identity source.
//!
//! `GET http://metadata.google.internal/computeMetadata/v1/instance/id`
//! with the required `Metadata-Flavor: Google` header. Response body is
//! the plaintext numeric instance ID.
//!
//! Authoritative reference:
//! [Compute Engine: About VM metadata](https://cloud.google.com/compute/docs/metadata/overview)
//! — specifies the `metadata.google.internal` endpoint, the
//! `Metadata-Flavor: Google` header requirement, and the
//! `/computeMetadata/v1/instance/id` path.

use crate::source::SourceKind;
use crate::sources::cloud::{CloudEndpoint, CloudMetadata};

/// GCP Compute Engine numeric instance ID via the metadata server.
pub type GcpMetadata<T> = CloudMetadata<GcpEndpoint, T>;

/// Endpoint descriptor for [`GcpMetadata`].
pub struct GcpEndpoint;

impl CloudEndpoint for GcpEndpoint {
    const DEBUG_NAME: &'static str = "GcpMetadata";
    const DEFAULT_BASE_URL: &'static str = "http://metadata.google.internal";
    const PATH: &'static str = "/computeMetadata/v1/instance/id";
    const KIND: SourceKind = SourceKind::GcpMetadata;

    fn headers() -> &'static [(&'static str, &'static str)] {
        &[("Metadata-Flavor", "Google")]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Source;
    use crate::sources::cloud::test_support::{StubTransport, ok, status};

    #[test]
    fn happy_path_returns_instance_id() {
        let stub = StubTransport::new(vec![ok("3709143138343389895")]);
        let source = GcpMetadata::new(stub);
        let probe = source.probe().unwrap().expect("should produce a probe");
        assert_eq!(probe.kind(), SourceKind::GcpMetadata);
        assert_eq!(probe.value(), "3709143138343389895");
    }

    #[test]
    fn sends_required_flavor_header() {
        let (stub, transport) = StubTransport::shared(vec![ok("42")]);
        let source = GcpMetadata::with_base_url(transport, "http://md.test");
        assert!(source.probe().unwrap().is_some());

        let requests = stub.requests();
        assert_eq!(requests.len(), 1);
        let (method, uri, headers) = &requests[0];
        assert_eq!(method, http::Method::GET);
        assert_eq!(uri, "http://md.test/computeMetadata/v1/instance/id");
        assert_eq!(headers.get("Metadata-Flavor").unwrap(), "Google");
    }

    #[test]
    fn non_2xx_returns_none() {
        let source = GcpMetadata::new(StubTransport::new(vec![status(404)]));
        assert!(source.probe().unwrap().is_none());
    }

    #[test]
    fn empty_body_returns_none() {
        let source = GcpMetadata::new(StubTransport::new(vec![ok("")]));
        assert!(source.probe().unwrap().is_none());
    }

    #[test]
    fn trailing_slash_on_base_url_does_not_double() {
        let (stub, transport) = StubTransport::shared(vec![ok("1")]);
        let source = GcpMetadata::with_base_url(transport, "http://md.test/");
        assert!(source.probe().unwrap().is_some());
        let requests = stub.requests();
        assert_eq!(
            requests[0].1,
            "http://md.test/computeMetadata/v1/instance/id"
        );
    }

    #[test]
    fn non_utf8_body_returns_none() {
        let response = http::Response::builder()
            .status(http::StatusCode::OK)
            .body(vec![0xff, 0xfe, 0xfd])
            .unwrap();
        let source = GcpMetadata::new(StubTransport::new(vec![response]));
        assert!(source.probe().unwrap().is_none());
    }
}
