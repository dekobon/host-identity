//! AWS EC2 Instance Metadata Service (`IMDSv2`) identity source.
//!
//! Fetches the EC2 instance ID via the `IMDSv2` token-authenticated flow:
//!
//! 1. `PUT /latest/api/token` with
//!    `X-aws-ec2-metadata-token-ttl-seconds: 21600` → session token.
//! 2. `GET /latest/dynamic/instance-identity/document` with
//!    `X-aws-ec2-metadata-token: <token>` → instance identity JSON.
//! 3. Extract the `instanceId` field.
//!
//! Authoritative references:
//!
//! - [AWS: Use IMDSv2](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/configuring-instance-metadata-service.html)
//!   — PUT/GET contract, TTL range (1 s – 21 600 s), required header names.
//! - [AWS: Instance identity documents](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/instance-identity-documents.html)
//!   — schema of the JSON document returned at
//!   `/latest/dynamic/instance-identity/document`, including the `instanceId`
//!   field this source extracts.
//!
//! The consumer supplies a [`crate::transport::HttpTransport`] — this crate
//! ships no HTTP client. Transport errors (connection refused, TLS failure,
//! timeout) and non-2xx responses are mapped to `Ok(None)` so the resolver
//! can fall through to the next source when not running on EC2. A 2xx
//! response with an unparseable body *is* a hard error.
//!
//! # Transport requirements (security)
//!
//! - The `IMDSv2` session token in the `X-aws-ec2-metadata-token` header is
//!   valid for up to 6 hours and permits full IMDS reads. Transports MUST
//!   treat this header as sensitive and MUST NOT log or forward it.
//! - Transports MUST NOT follow HTTP redirects for IMDS requests. A
//!   redirect off the link-local endpoint would leak the token to an
//!   off-host destination.
//! - A short per-request timeout is strongly recommended (single-digit
//!   seconds). Off-EC2 hosts never answer `169.254.169.254`, so without
//!   a timeout the resolver blocks indefinitely.

use crate::error::Error;
use crate::source::{Probe, Source, SourceKind};
use crate::sources::util::{normalize, trim_trailing_slashes};
use crate::transport::HttpTransport;

const DEFAULT_BASE_URL: &str = "http://169.254.169.254";
const TOKEN_PATH: &str = "/latest/api/token";
const DOCUMENT_PATH: &str = "/latest/dynamic/instance-identity/document";
const TOKEN_TTL_HEADER: &str = "X-aws-ec2-metadata-token-ttl-seconds";
const TOKEN_HEADER: &str = "X-aws-ec2-metadata-token";
const TOKEN_TTL_SECONDS: &str = "21600";

/// AWS EC2 instance ID via `IMDSv2`.
pub struct AwsImds<T> {
    transport: T,
    base_url: String,
}

impl<T> AwsImds<T> {
    /// Use the link-local IMDS endpoint at `http://169.254.169.254`.
    pub fn new(transport: T) -> Self {
        Self::with_base_url(transport, DEFAULT_BASE_URL)
    }

    /// Use a caller-supplied base URL. Useful for tests and for VPCs that
    /// route IMDS through a proxy.
    ///
    /// Any trailing `/` is trimmed so that concatenation with the IMDS
    /// token / document paths never produces `//`.
    pub fn with_base_url(transport: T, base_url: impl Into<String>) -> Self {
        Self {
            transport,
            base_url: trim_trailing_slashes(base_url),
        }
    }
}

impl<T> std::fmt::Debug for AwsImds<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwsImds")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl<T: HttpTransport + 'static> Source for AwsImds<T> {
    fn kind(&self) -> SourceKind {
        SourceKind::AwsImds
    }

    fn probe(&self) -> Result<Option<Probe>, Error> {
        let Some(token) = fetch_token(&self.transport, &self.base_url) else {
            return Ok(None);
        };
        let Some(document) = fetch_document(&self.transport, &self.base_url, &token) else {
            return Ok(None);
        };
        let instance_id = extract_instance_id(&document).ok_or_else(|| Error::Platform {
            source_kind: SourceKind::AwsImds,
            reason: "instance-identity document missing `instanceId` field".to_owned(),
        })?;
        Ok(normalize(&instance_id).map(|v| Probe::new(SourceKind::AwsImds, v)))
    }
}

fn fetch_token<T: HttpTransport>(transport: &T, base_url: &str) -> Option<String> {
    let request = http::Request::builder()
        .method(http::Method::PUT)
        .uri(format!("{base_url}{TOKEN_PATH}"))
        .header(TOKEN_TTL_HEADER, TOKEN_TTL_SECONDS)
        .body(Vec::new())
        .ok()?;
    send_plaintext(transport, request, "token")
}

fn fetch_document<T: HttpTransport>(transport: &T, base_url: &str, token: &str) -> Option<String> {
    let request = http::Request::builder()
        .method(http::Method::GET)
        .uri(format!("{base_url}{DOCUMENT_PATH}"))
        .header(TOKEN_HEADER, token)
        .body(Vec::new())
        .ok()?;
    send_plaintext(transport, request, "document")
}

fn send_plaintext<T: HttpTransport>(
    transport: &T,
    request: http::Request<Vec<u8>>,
    label: &str,
) -> Option<String> {
    let response = transport.send(request).ok()?;
    if !response.status().is_success() {
        log::debug!("aws-imds: {label} endpoint returned {}", response.status());
        return None;
    }
    std::str::from_utf8(response.body()).ok().map(str::to_owned)
}

/// Extract a top-level `"instanceId": "..."` string value.
///
/// The IMDS identity document is a flat JSON object with no escape sequences
/// in its field values, so a substring scan is sufficient and avoids a
/// `serde_json` dependency. Do not reuse this for general JSON parsing — it
/// does not handle string escapes or nested objects.
fn extract_instance_id(json: &str) -> Option<String> {
    // Scan for `"instanceId"` preceded by a JSON structural boundary
    // (`{` or `,`, optionally with whitespace between). This prevents
    // a substring match inside an embedded string value from winning
    // over the real top-level key.
    let key = "\"instanceId\"";
    let mut cursor = 0;
    let start = loop {
        let rel = json[cursor..].find(key)?;
        let abs = cursor + rel;
        if is_at_top_level_boundary(json, abs) {
            break abs;
        }
        cursor = abs + key.len();
    };
    let after_key = &json[start + key.len()..];
    let colon = after_key.find(':')?;
    let after_colon = after_key[colon + 1..].trim_start();
    let after_open_quote = after_colon.strip_prefix('"')?;
    let close = after_open_quote.find('"')?;
    Some(after_open_quote[..close].to_owned())
}

fn is_at_top_level_boundary(json: &str, key_start: usize) -> bool {
    json[..key_start]
        .bytes()
        .rev()
        .find(|b| !b.is_ascii_whitespace())
        .is_some_and(|b| b == b'{' || b == b',')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::cloud::test_support::{
        StubTransport, ok as ok_response, status as status_response,
    };

    const IID_DOC: &str = r#"{
        "accountId": "123456789012",
        "architecture": "x86_64",
        "availabilityZone": "us-east-1a",
        "instanceId": "i-0abc1234def567890",
        "instanceType": "t3.small",
        "region": "us-east-1"
    }"#;

    #[test]
    fn happy_path_returns_instance_id() {
        let stub = StubTransport::new(vec![ok_response("AQAAA-token-bytes"), ok_response(IID_DOC)]);
        let source = AwsImds::new(stub);
        let probe = source.probe().unwrap().expect("should produce a probe");
        assert_eq!(probe.kind(), SourceKind::AwsImds);
        assert_eq!(probe.value(), "i-0abc1234def567890");
    }

    #[test]
    fn requests_follow_imdsv2_contract() {
        let (stub, transport) =
            StubTransport::shared(vec![ok_response("tok"), ok_response(IID_DOC)]);
        let source = AwsImds::with_base_url(transport, "http://imds.test");
        assert!(source.probe().unwrap().is_some());

        let requests = stub.requests();
        assert_eq!(requests.len(), 2);

        let (ref method, ref uri, ref headers) = requests[0];
        assert_eq!(method, http::Method::PUT);
        assert_eq!(uri, "http://imds.test/latest/api/token");
        assert_eq!(headers.get(TOKEN_TTL_HEADER).unwrap(), TOKEN_TTL_SECONDS);

        let (ref method, ref uri, ref headers) = requests[1];
        assert_eq!(method, http::Method::GET);
        assert_eq!(
            uri,
            "http://imds.test/latest/dynamic/instance-identity/document"
        );
        assert_eq!(headers.get(TOKEN_HEADER).unwrap(), "tok");
    }

    #[test]
    fn token_non_2xx_returns_none() {
        let stub = StubTransport::new(vec![status_response(401)]);
        let source = AwsImds::new(stub);
        assert!(source.probe().unwrap().is_none());
    }

    #[test]
    fn document_non_2xx_returns_none() {
        let stub = StubTransport::new(vec![ok_response("tok"), status_response(404)]);
        let source = AwsImds::new(stub);
        assert!(source.probe().unwrap().is_none());
    }

    #[test]
    fn transport_error_returns_none() {
        #[derive(Debug)]
        struct FakeErr;
        impl std::fmt::Display for FakeErr {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("nope")
            }
        }
        impl std::error::Error for FakeErr {}

        let transport = |_req: http::Request<Vec<u8>>| -> Result<http::Response<Vec<u8>>, FakeErr> {
            Err(FakeErr)
        };
        let source = AwsImds::new(transport);
        assert!(source.probe().unwrap().is_none());
    }

    #[test]
    fn missing_instance_id_field_errors() {
        let document = r#"{"accountId": "123", "region": "us-east-1"}"#;
        let stub = StubTransport::new(vec![ok_response("tok"), ok_response(document)]);
        let source = AwsImds::new(stub);
        let err = source.probe().expect_err("missing field must error");
        assert!(matches!(
            &err,
            Error::Platform { source_kind, reason }
                if *source_kind == SourceKind::AwsImds && reason.contains("instanceId")
        ));
    }

    #[test]
    fn extract_instance_id_parses_simple_document() {
        assert_eq!(
            extract_instance_id(IID_DOC).as_deref(),
            Some("i-0abc1234def567890")
        );
    }

    #[test]
    fn extract_instance_id_returns_none_when_field_absent() {
        assert_eq!(extract_instance_id(r#"{"region": "us-east-1"}"#), None);
    }

    #[test]
    fn extract_instance_id_skips_key_embedded_in_string_value() {
        // The literal substring `"instanceId"` embedded inside an earlier
        // tag value must not win — only a `"instanceId"` preceded by a
        // JSON structural boundary (`{` or `,`) counts as the real key.
        let doc = r#"{"note": "x\"instanceId\":\"i-attacker\"", "instanceId": "i-real"}"#;
        assert_eq!(extract_instance_id(doc).as_deref(), Some("i-real"));
    }

    #[test]
    fn extract_instance_id_rejects_when_only_match_is_embedded() {
        // With no real top-level key, the scanner must not fall back to
        // the embedded occurrence.
        let doc = r#"{"note": "x,\"instanceId\":\"i-attacker\""}"#;
        assert_eq!(extract_instance_id(doc), None);
    }

    #[test]
    fn extract_instance_id_tolerates_whitespace_around_colon() {
        let doc = r#"{"instanceId"   :   "i-xyz"}"#;
        assert_eq!(extract_instance_id(doc).as_deref(), Some("i-xyz"));
    }
}
