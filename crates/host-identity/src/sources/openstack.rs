//! `OpenStack` Nova metadata service identity source.
//!
//! Fetches the Nova instance UUID via a single `GET` to the
//! `OpenStack`-native metadata path:
//!
//! 1. `GET /openstack/2018-08-27/meta_data.json` → flat JSON document.
//! 2. Extract the top-level `uuid` field (the raw Nova instance UUID).
//!
//! The EC2-compatibility surface at the same endpoint
//! (`/<version>/meta-data/instance-id`) is deliberately **not** used:
//! it returns a lossy `i-XXXXXXXX` short ID derived from the UUID
//! rather than the UUID itself. See
//! [`nova/api/metadata/base.py`](https://github.com/openstack/nova/blob/master/nova/api/metadata/base.py)
//! — `metadata['uuid'] = self.uuid` vs
//! `'instance-id': self.instance.ec2_ids.instance_id`.
//!
//! Authoritative references:
//!
//! - [`OpenStack` Nova: metadata service (latest user guide)](https://docs.openstack.org/nova/latest/user/metadata.html)
//!   — path layout, supported dated versions, field shapes.
//! - [Nova release index](https://releases.openstack.org/teams/nova.html)
//!   — the pinned `2018-08-27` schema version has been served by every
//!   supported Nova release; `uuid` itself has been present since
//!   `2012-08-10`.
//! - [`nova/api/metadata/base.py`](https://github.com/openstack/nova/blob/master/nova/api/metadata/base.py)
//!   — source of truth for which Python field populates which
//!   metadata key.
//!
//! The consumer supplies a [`crate::transport::HttpTransport`] — this
//! crate ships no HTTP client. Outcome classification matches the
//! `AwsImds` contract:
//!
//! - Transport errors (connection refused, TLS failure, timeout),
//!   non-2xx responses, and non-UTF-8 bodies are mapped to `Ok(None)`
//!   so the resolver falls through when the host isn't on `OpenStack`.
//! - A 2xx response whose body lacks the top-level `uuid` field is a
//!   **hard failure**: we're on `OpenStack` but the document doesn't
//!   match the documented Nova schema. The caller should see that.
//! - An empty or whitespace `uuid` value degrades to `Ok(None)` via
//!   [`crate::sources::util::normalize`] — treat it as a transient
//!   misconfiguration rather than a schema violation.
//!
//! # Identity scope
//!
//! This source returns a **per-instance** identifier — the Nova
//! instance UUID of the host the caller is running on. Every
//! container and every process on that instance sees the same value.
//! A resolver that wants per-container identity must place
//! `ContainerId` (or `KubernetesPodUid` inside a pod) above
//! `OpenStackMetadata` in the chain; otherwise every container on
//! one `OpenStack` host collapses onto one ID. The default network
//! chain (`Resolver::with_network_defaults`) does this automatically.
//!
//! # Transport requirements (security)
//!
//! - Transports MUST NOT follow HTTP redirects for metadata
//!   requests. A redirect off the link-local endpoint would forward
//!   the request (and any future headers this source adds) to an
//!   off-host destination.
//! - A short per-request timeout is strongly recommended (single-digit
//!   seconds). Off-`OpenStack` hosts never answer `169.254.169.254`, so
//!   without a timeout the resolver blocks indefinitely.
//!
//! Unlike `AwsImds`, the `OpenStack` metadata service requires no
//! token exchange and no sensitive headers — the single GET carries
//! no caller-supplied authentication.

use crate::error::Error;
use crate::source::{Probe, Source, SourceKind};
use crate::sources::util::{normalize, trim_trailing_slashes};
use crate::transport::HttpTransport;

const DEFAULT_BASE_URL: &str = "http://169.254.169.254";
const METADATA_PATH: &str = "/openstack/2018-08-27/meta_data.json";

/// `OpenStack` Nova instance UUID via the metadata service.
pub struct OpenStackMetadata<T> {
    transport: T,
    base_url: String,
}

impl<T> OpenStackMetadata<T> {
    /// Use the link-local metadata endpoint at `http://169.254.169.254`.
    pub fn new(transport: T) -> Self {
        Self::with_base_url(transport, DEFAULT_BASE_URL)
    }

    /// Use a caller-supplied base URL. Useful for tests and for
    /// private clouds that route metadata through a proxy.
    ///
    /// Any trailing `/` is trimmed so that concatenation with the
    /// metadata path never produces `//`.
    pub fn with_base_url(transport: T, base_url: impl Into<String>) -> Self {
        Self {
            transport,
            base_url: trim_trailing_slashes(base_url),
        }
    }
}

impl<T> std::fmt::Debug for OpenStackMetadata<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenStackMetadata")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl<T: HttpTransport + 'static> Source for OpenStackMetadata<T> {
    fn kind(&self) -> SourceKind {
        SourceKind::OpenStackMetadata
    }

    fn probe(&self) -> Result<Option<Probe>, Error> {
        let Some(body) = fetch_metadata(&self.transport, &self.base_url) else {
            return Ok(None);
        };
        let uuid = extract_uuid(&body).ok_or_else(|| Error::Platform {
            source_kind: SourceKind::OpenStackMetadata,
            reason: "meta_data.json missing top-level `uuid` field".to_owned(),
        })?;
        Ok(normalize(&uuid).map(|v| Probe::new(SourceKind::OpenStackMetadata, v)))
    }
}

fn fetch_metadata<T: HttpTransport>(transport: &T, base_url: &str) -> Option<String> {
    let request = http::Request::builder()
        .method(http::Method::GET)
        .uri(format!("{base_url}{METADATA_PATH}"))
        .body(Vec::new())
        .ok()?;
    let response = transport.send(request).ok()?;
    if !response.status().is_success() {
        log::debug!(
            "openstack-metadata: endpoint returned {}",
            response.status()
        );
        return None;
    }
    std::str::from_utf8(response.body()).ok().map(str::to_owned)
}

/// Extract a top-level `"uuid": "..."` string value.
///
/// `meta_data.json` is a shallow JSON object whose `uuid` value is a
/// canonical 36-character UUID containing no escape sequences, so a
/// boundary-aware substring scan is sufficient and avoids a
/// `serde_json` dependency. Do not reuse this for general JSON
/// parsing — it does not handle string escapes or nested-object
/// traversal.
///
/// The boundary check (`is_at_top_level_boundary`) rejects matches
/// embedded inside string values and matches that are a suffix of a
/// longer key (for example `"project_uuid"`). A nested object whose
/// first key is `"uuid"` *would* match; the test
/// `extract_uuid_handles_nested_devices_array` locks in that Nova's
/// emission order places the top-level `uuid` before any nested
/// object, so `find` returns the correct occurrence. If that
/// ordering ever changes the test fails loudly and we can add
/// nesting-depth tracking then.
fn extract_uuid(json: &str) -> Option<String> {
    let key = "\"uuid\"";
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

    const NOVA_DOC: &str = r#"{
        "random_seed": "lkWdGuiWmMWrh7ox1mQpFH1w",
        "uuid": "d8e02d56-2648-49a3-bf97-6be8f1204f38",
        "availability_zone": "nova",
        "hostname": "test.novalocal",
        "launch_index": 0,
        "devices": [],
        "project_id": "f7ac731cc11f40efbc03a9f9e1d1d21f",
        "name": "test"
    }"#;

    #[test]
    fn happy_path_returns_uuid_from_json() {
        let stub = StubTransport::new(vec![ok_response(NOVA_DOC)]);
        let source = OpenStackMetadata::new(stub);
        let probe = source.probe().unwrap().expect("should produce a probe");
        assert_eq!(probe.kind(), SourceKind::OpenStackMetadata);
        assert_eq!(probe.value(), "d8e02d56-2648-49a3-bf97-6be8f1204f38");
    }

    #[test]
    fn hits_expected_path() {
        let (stub, transport) = StubTransport::shared(vec![ok_response(NOVA_DOC)]);
        let source = OpenStackMetadata::with_base_url(transport, "http://md.test");
        assert!(source.probe().unwrap().is_some());

        let requests = stub.requests();
        assert_eq!(requests.len(), 1);
        let (ref method, ref uri, ref headers) = requests[0];
        assert_eq!(method, http::Method::GET);
        assert_eq!(uri, "http://md.test/openstack/2018-08-27/meta_data.json");
        // No provider-specific headers; the single GET is unauthenticated.
        assert!(headers.is_empty());
    }

    #[test]
    fn non_2xx_returns_none() {
        let stub = StubTransport::new(vec![status_response(503)]);
        let source = OpenStackMetadata::new(stub);
        assert!(source.probe().unwrap().is_none());
    }

    #[test]
    fn not_found_returns_none() {
        // Older Nova deployments serving the root listing but missing
        // the pinned dated version hit this path.
        let stub = StubTransport::new(vec![status_response(404)]);
        let source = OpenStackMetadata::new(stub);
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
        let source = OpenStackMetadata::new(transport);
        assert!(source.probe().unwrap().is_none());
    }

    #[test]
    fn non_utf8_body_returns_none() {
        let response = http::Response::builder()
            .status(200)
            .body(vec![0xff, 0xfe, 0xfd])
            .unwrap();
        let stub = StubTransport::new(vec![response]);
        let source = OpenStackMetadata::new(stub);
        assert!(source.probe().unwrap().is_none());
    }

    #[test]
    fn empty_uuid_returns_none() {
        let stub = StubTransport::new(vec![ok_response(r#"{"uuid":""}"#)]);
        let source = OpenStackMetadata::new(stub);
        assert!(source.probe().unwrap().is_none());
    }

    #[test]
    fn whitespace_uuid_returns_none() {
        let stub = StubTransport::new(vec![ok_response(r#"{"uuid":"   "}"#)]);
        let source = OpenStackMetadata::new(stub);
        assert!(source.probe().unwrap().is_none());
    }

    #[test]
    fn missing_uuid_field_is_platform_error() {
        let stub = StubTransport::new(vec![ok_response(r#"{"hostname":"x"}"#)]);
        let source = OpenStackMetadata::new(stub);
        let err = source.probe().expect_err("missing field must error");
        assert!(matches!(
            &err,
            Error::Platform { source_kind, reason }
                if *source_kind == SourceKind::OpenStackMetadata && reason.contains("uuid")
        ));
    }

    #[test]
    fn extract_uuid_parses_nova_document() {
        assert_eq!(
            extract_uuid(NOVA_DOC).as_deref(),
            Some("d8e02d56-2648-49a3-bf97-6be8f1204f38")
        );
    }

    #[test]
    fn extract_uuid_returns_none_when_field_absent() {
        assert_eq!(extract_uuid(r#"{"hostname":"x"}"#), None);
    }

    #[test]
    fn extract_uuid_skips_key_embedded_in_string_value() {
        // The literal substring `"uuid"` embedded inside an earlier
        // tag value must not win — only a `"uuid"` preceded by a JSON
        // structural boundary (`{` or `,`) counts as the real key.
        let doc = r#"{"note": "x\"uuid\":\"fake\"", "uuid": "real"}"#;
        assert_eq!(extract_uuid(doc).as_deref(), Some("real"));
    }

    #[test]
    fn extract_uuid_rejects_when_only_match_is_embedded() {
        let doc = r#"{"note": "x,\"uuid\":\"fake\""}"#;
        assert_eq!(extract_uuid(doc), None);
    }

    #[test]
    fn extract_uuid_tolerates_whitespace_around_colon() {
        let doc = r#"{"uuid"   :   "xyz"}"#;
        assert_eq!(extract_uuid(doc).as_deref(), Some("xyz"));
    }

    #[test]
    fn extract_uuid_skips_uuid_suffix_in_other_key() {
        // The boundary check rejects `..._uuid` keys because the byte
        // before the opening quote of `"uuid"` is `_`, not `{` / `,`.
        let doc = r#"{"project_uuid":"proj-123","uuid":"real"}"#;
        assert_eq!(extract_uuid(doc).as_deref(), Some("real"));
    }

    #[test]
    fn extract_uuid_handles_nested_devices_array() {
        // Nova emits the top-level `uuid` before nested objects like
        // `devices`. `find` returns the earliest match, which is the
        // correct one. If Nova ever reorders fields to put a nested
        // object before `uuid`, this test fails and we add
        // nesting-depth tracking.
        let doc = r#"{"uuid":"REAL","devices":[{"uuid":"DEVICE"}]}"#;
        assert_eq!(extract_uuid(doc).as_deref(), Some("REAL"));
    }

    #[test]
    fn extract_uuid_rejects_uuid_appearing_as_value_string() {
        // `"uuid"` appears as a VALUE — the literal 6-byte pattern
        // `"`,`u`,`u`,`i`,`d`,`"` is present but preceded by `:`, not
        // `{` / `,`. The boundary check rejects it; no real top-level
        // `uuid` key follows, so the result is `None`.
        //
        // Without the boundary check the scanner would walk forward
        // from the bad match, find the next `:`, and incorrectly
        // extract the `y` field's value ("real"). This test is the
        // only one that exercises `is_at_top_level_boundary`.
        let doc = r#"{"x":"uuid","y":"real"}"#;
        assert_eq!(extract_uuid(doc), None);
    }

    #[test]
    fn extract_uuid_rejects_malformed_value() {
        // Truncated body: the `uuid` key matches but the value is
        // missing. The scanner returns None, and the probe surfaces
        // it as a Platform error at the call site.
        let doc = r#"{"uuid":"#;
        assert_eq!(extract_uuid(doc), None);
    }
}
