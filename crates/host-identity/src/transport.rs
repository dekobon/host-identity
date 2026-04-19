//! HTTP transport abstraction for network-derived identity sources.
//!
//! Built-in cloud sources (e.g. [`crate::sources::AwsImds`]) are generic over
//! a transport the consumer provides. The crate ships no HTTP client of its
//! own — picking one (sync, async, TLS backend, connection pool, …) is the
//! consumer's call.
//!
//! Adapt an existing client by implementing [`HttpTransport`] on a newtype,
//! or pass a closure — a blanket impl accepts any
//! `Fn(Request) -> Result<Response, _>`.
//!
//! ```
//! # #[cfg(feature = "_transport")] fn compile_check() {
//! use host_identity::transport::HttpTransport;
//!
//! // Stand-in types — a real adapter uses ureq / reqwest / hyper.
//! struct MyClient;
//! #[derive(Debug)]
//! struct MyError;
//! impl std::fmt::Display for MyError {
//!     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//!         f.write_str("my transport error")
//!     }
//! }
//! impl std::error::Error for MyError {}
//!
//! impl HttpTransport for MyClient {
//!     type Error = MyError;
//!     fn send(
//!         &self,
//!         _request: http::Request<Vec<u8>>,
//!     ) -> Result<http::Response<Vec<u8>>, Self::Error> {
//!         // In a real impl: translate http::Request to your client's
//!         // request type, perform the call, translate the response
//!         // back. The crate's cloud sources pass the request in
//!         // populated; you translate bytes both ways and surface
//!         // network/TLS/timeout errors as `Err`.
//!         Ok(http::Response::builder()
//!             .status(200)
//!             .body(b"example-id".to_vec())
//!             .unwrap())
//!     }
//! }
//! # }
//! ```

/// Synchronous HTTP transport.
///
/// The library owns the protocol (URLs, headers, response parsing); the
/// transport only moves bytes. Async clients are supported by wrapping their
/// call in `block_on` inside [`HttpTransport::send`].
pub trait HttpTransport: Send + Sync {
    /// Transport-specific error type.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Perform one request/response round trip.
    ///
    /// Implementations should surface network errors, TLS errors, and
    /// timeouts as `Err`. A non-2xx HTTP status is *not* an error — return
    /// the [`http::Response`] and let the caller inspect
    /// [`http::Response::status`].
    fn send(&self, request: http::Request<Vec<u8>>)
    -> Result<http::Response<Vec<u8>>, Self::Error>;
}

/// Blanket impl so any `Fn(Request) -> Result<Response, E>` is a transport.
impl<F, E> HttpTransport for F
where
    F: Fn(http::Request<Vec<u8>>) -> Result<http::Response<Vec<u8>>, E> + Send + Sync,
    E: std::error::Error + Send + Sync + 'static,
{
    type Error = E;

    fn send(
        &self,
        request: http::Request<Vec<u8>>,
    ) -> Result<http::Response<Vec<u8>>, Self::Error> {
        self(request)
    }
}
