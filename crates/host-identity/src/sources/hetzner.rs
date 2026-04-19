//! Hetzner Cloud instance-metadata identity source.
//!
//! `GET http://169.254.169.254/hetzner/v1/metadata/instance-id` — plaintext
//! numeric server ID, no special headers.
//!
//! Authoritative reference:
//! [Hetzner Cloud: Server metadata](https://docs.hetzner.cloud/#server-metadata)
//! — documents the `169.254.169.254` endpoint and the
//! `/hetzner/v1/metadata/instance-id` leaf.

use crate::source::SourceKind;
use crate::sources::cloud::{CloudEndpoint, CloudMetadata};

/// Hetzner Cloud numeric server ID.
pub type HetznerMetadata<T> = CloudMetadata<HetznerEndpoint, T>;

/// Endpoint descriptor for [`HetznerMetadata`].
pub struct HetznerEndpoint;

impl CloudEndpoint for HetznerEndpoint {
    const DEBUG_NAME: &'static str = "HetznerMetadata";
    const DEFAULT_BASE_URL: &'static str = "http://169.254.169.254";
    const PATH: &'static str = "/hetzner/v1/metadata/instance-id";
    const KIND: SourceKind = SourceKind::HetznerMetadata;

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
    fn happy_path_returns_server_id() {
        let source = HetznerMetadata::new(StubTransport::new(vec![ok("42001337")]));
        let probe = source.probe().unwrap().expect("should produce a probe");
        assert_eq!(probe.kind(), SourceKind::HetznerMetadata);
        assert_eq!(probe.value(), "42001337");
    }

    #[test]
    fn hits_expected_path() {
        let (stub, transport) = StubTransport::shared(vec![ok("1")]);
        let source = HetznerMetadata::with_base_url(transport, "http://hz.test");
        assert!(source.probe().unwrap().is_some());

        let requests = stub.requests();
        assert_eq!(requests.len(), 1);
        let (_, uri, _) = &requests[0];
        assert_eq!(uri, "http://hz.test/hetzner/v1/metadata/instance-id");
    }

    #[test]
    fn non_2xx_returns_none() {
        let source = HetznerMetadata::new(StubTransport::new(vec![status(503)]));
        assert!(source.probe().unwrap().is_none());
    }
}
