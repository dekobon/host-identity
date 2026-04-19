//! `DigitalOcean` Droplet metadata identity source.
//!
//! `GET http://169.254.169.254/metadata/v1/id` — plaintext droplet ID, no
//! special headers required.
//!
//! Authoritative reference:
//! [DigitalOcean: Droplet metadata API reference](https://docs.digitalocean.com/reference/api/metadata-api/)
//! — documents the `169.254.169.254` link-local endpoint, the `/metadata/v1/`
//! tree, and the `/metadata/v1/id` plaintext numeric droplet ID.

use crate::source::SourceKind;
use crate::sources::cloud::{CloudEndpoint, CloudMetadata};

/// `DigitalOcean` Droplet numeric ID.
pub type DigitalOceanMetadata<T> = CloudMetadata<DigitalOceanEndpoint, T>;

/// Endpoint descriptor for [`DigitalOceanMetadata`].
pub struct DigitalOceanEndpoint;

impl CloudEndpoint for DigitalOceanEndpoint {
    const DEBUG_NAME: &'static str = "DigitalOceanMetadata";
    const DEFAULT_BASE_URL: &'static str = "http://169.254.169.254";
    const PATH: &'static str = "/metadata/v1/id";
    const KIND: SourceKind = SourceKind::DigitalOceanMetadata;

    fn headers() -> &'static [(&'static str, &'static str)] {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Source;
    use crate::sources::cloud::test_support::{StubTransport, ok, status};

    #[test]
    fn happy_path_returns_droplet_id() {
        let source = DigitalOceanMetadata::new(StubTransport::new(vec![ok("1234567")]));
        let probe = source.probe().unwrap().expect("should produce a probe");
        assert_eq!(probe.kind(), SourceKind::DigitalOceanMetadata);
        assert_eq!(probe.value(), "1234567");
    }

    #[test]
    fn hits_expected_path_with_no_extra_headers() {
        let (stub, transport) = StubTransport::shared(vec![ok("42")]);
        let source = DigitalOceanMetadata::with_base_url(transport, "http://do.test");
        assert!(source.probe().unwrap().is_some());

        let requests = stub.requests();
        assert_eq!(requests.len(), 1);
        let (method, uri, headers) = &requests[0];
        assert_eq!(method, http::Method::GET);
        assert_eq!(uri, "http://do.test/metadata/v1/id");
        assert!(headers.is_empty());
    }

    #[test]
    fn non_2xx_returns_none() {
        let source = DigitalOceanMetadata::new(StubTransport::new(vec![status(404)]));
        assert!(source.probe().unwrap().is_none());
    }
}
