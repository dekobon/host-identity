//! Shared plumbing for cloud-metadata identity sources.
//!
//! Every plaintext-body provider (every one except AWS, which has a
//! two-step token dance) follows the same shape: one GET to a link-local
//! or private-DNS endpoint, with zero or more provider-specific headers,
//! returning a plaintext instance identifier. That shape is expressed once
//! here as [`CloudMetadata<E, T>`], parameterised by a [`CloudEndpoint`]
//! type that carries the URL, headers, source kind, and `Debug` label.
//!
//! # Transport requirements (security)
//!
//! Every source built on [`CloudMetadata`] contacts a link-local or
//! private-DNS endpoint that answers only on the specific cloud provider.
//! To avoid exfiltrating provider-specific headers (e.g. Azure's
//! `Metadata: true` fingerprint) or the request path, transports MUST:
//!
//! - refuse to follow HTTP redirects; a compromised or misconfigured
//!   endpoint returning a 3xx to an attacker-controlled host would
//!   forward those headers off the cloud;
//! - enforce a short per-request timeout (single-digit seconds). Off-
//!   cloud hosts never answer these endpoints, so the timeout directly
//!   bounds resolver latency when a source has no path to its endpoint.

use std::marker::PhantomData;

use crate::error::Error;
use crate::source::{Probe, Source, SourceKind};
use crate::sources::util::{normalize, trim_trailing_slashes};
use crate::transport::HttpTransport;

/// Provider-specific constants for a plaintext metadata endpoint.
///
/// One zero-sized type per provider. Keeps the wire details next to the
/// provider and out of the generic [`CloudMetadata`] impl.
pub trait CloudEndpoint: Send + Sync + 'static {
    /// Short struct name used in `Debug` output. Keeps
    /// `format!("{:?}", GcpMetadata)` readable rather than leaking
    /// `CloudMetadata<GcpEndpoint, _>`.
    const DEBUG_NAME: &'static str;
    /// Default endpoint base URL.
    const DEFAULT_BASE_URL: &'static str;
    /// Path (and any query string) appended to the base URL.
    const PATH: &'static str;
    /// [`SourceKind`] label for the resolved probe.
    const KIND: SourceKind;

    /// Provider-required headers. Return `&[]` when none are needed.
    fn headers() -> &'static [(&'static str, &'static str)];
}

/// Generic cloud-metadata source over a transport and an endpoint descriptor.
pub struct CloudMetadata<E, T> {
    transport: T,
    base_url: String,
    _endpoint: PhantomData<fn() -> E>,
}

impl<E: CloudEndpoint, T> CloudMetadata<E, T> {
    /// Use the endpoint's default base URL.
    pub fn new(transport: T) -> Self {
        Self::with_base_url(transport, E::DEFAULT_BASE_URL)
    }

    /// Use a caller-supplied base URL (tests, proxies).
    ///
    /// Any trailing `/` is trimmed so that concatenation with the
    /// endpoint's leading-slash path never produces `//`.
    pub fn with_base_url(transport: T, base_url: impl Into<String>) -> Self {
        Self {
            transport,
            base_url: trim_trailing_slashes(base_url),
            _endpoint: PhantomData,
        }
    }
}

impl<E: CloudEndpoint, T> std::fmt::Debug for CloudMetadata<E, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(E::DEBUG_NAME)
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl<E: CloudEndpoint, T: HttpTransport + 'static> Source for CloudMetadata<E, T> {
    fn kind(&self) -> SourceKind {
        E::KIND
    }

    fn probe(&self) -> Result<Option<Probe>, Error> {
        let url = format!("{}{}", self.base_url, E::PATH);
        let body = fetch_plaintext_id(&self.transport, &url, E::KIND, E::headers());
        Ok(body
            .as_deref()
            .and_then(normalize)
            .map(|v| Probe::new(E::KIND, v)))
    }
}

/// Issue a single `GET` and return the body as a UTF-8 string if the
/// response is 2xx. Transport errors and non-2xx responses produce `None`
/// so the resolver can fall through to the next source when the host is
/// clearly not on this provider.
fn fetch_plaintext_id<T: HttpTransport>(
    transport: &T,
    url: &str,
    kind: SourceKind,
    headers: &[(&str, &str)],
) -> Option<String> {
    let mut builder = http::Request::builder().method(http::Method::GET).uri(url);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = builder.body(Vec::new()).ok()?;
    let response = transport.send(request).ok()?;
    if !response.status().is_success() {
        // Log the source kind and status, not the URL — future endpoints
        // may interpolate tokens or other sensitive data into the path.
        log::debug!("{kind}: endpoint returned {}", response.status());
        return None;
    }
    std::str::from_utf8(response.body()).ok().map(str::to_owned)
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Canned-response `HttpTransport` used by every cloud-source test.

    use std::collections::VecDeque;
    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};

    use crate::transport::HttpTransport;

    pub(crate) struct StubTransport {
        inner: Mutex<Inner>,
    }

    struct Inner {
        responses: VecDeque<http::Response<Vec<u8>>>,
        requests: Vec<(http::Method, http::Uri, http::HeaderMap)>,
    }

    impl StubTransport {
        pub(crate) fn new(responses: Vec<http::Response<Vec<u8>>>) -> Self {
            Self {
                inner: Mutex::new(Inner {
                    responses: responses.into(),
                    requests: Vec::new(),
                }),
            }
        }

        pub(crate) fn requests(&self) -> Vec<(http::Method, http::Uri, http::HeaderMap)> {
            self.inner.lock().unwrap().requests.clone()
        }

        /// Build a shared stub and an owned transport closure wired to it.
        /// Tests use the returned `Arc` to assert on captured requests after
        /// the source has been dropped.
        pub(crate) fn shared(
            responses: Vec<http::Response<Vec<u8>>>,
        ) -> (Arc<Self>, impl HttpTransport + 'static) {
            let stub = Arc::new(Self::new(responses));
            let handle = Arc::clone(&stub);
            let transport = move |req: http::Request<Vec<u8>>| handle.send(req);
            (stub, transport)
        }
    }

    impl HttpTransport for StubTransport {
        type Error = Infallible;
        fn send(
            &self,
            request: http::Request<Vec<u8>>,
        ) -> Result<http::Response<Vec<u8>>, Self::Error> {
            let mut guard = self.inner.lock().unwrap();
            let response = guard
                .responses
                .pop_front()
                .expect("stub transport ran out of canned responses");
            guard.requests.push((
                request.method().clone(),
                request.uri().clone(),
                request.headers().clone(),
            ));
            Ok(response)
        }
    }

    pub(crate) fn ok(body: &str) -> http::Response<Vec<u8>> {
        http::Response::builder()
            .status(http::StatusCode::OK)
            .body(body.as_bytes().to_vec())
            .unwrap()
    }

    pub(crate) fn status(code: u16) -> http::Response<Vec<u8>> {
        http::Response::builder()
            .status(code)
            .body(Vec::new())
            .unwrap()
    }
}
