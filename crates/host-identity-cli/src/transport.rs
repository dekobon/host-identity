//! Blocking `HttpTransport` implementation backed by `ureq`.

use std::io::Read;
use std::time::Duration;

use host_identity::transport::HttpTransport;

/// Upper bound on metadata response bodies. Cloud and k8s identity
/// endpoints return small strings (UUIDs, a few-KiB JSON blobs). This
/// cap exists as a `DoS` guard; exceeding it is surfaced as an error so
/// callers never silently see a truncated body.
const MAX_RESPONSE_BYTES: u64 = 1 << 20;

/// Default per-request timeout for cloud-metadata endpoints. On-cloud
/// responses return in single-digit milliseconds; off-cloud the call
/// hangs until this fires, so this directly bounds CLI latency when a
/// source has no path to its endpoint.
pub(crate) const DEFAULT_NETWORK_TIMEOUT: Duration = Duration::from_millis(750);

#[derive(Clone)]
pub(crate) struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    pub(crate) fn with_timeout(timeout: Duration) -> Self {
        // Cap connect at half the overall budget so a slow TCP handshake
        // still leaves time for the response.
        let connect = timeout / 2;
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(connect)
            .timeout(timeout)
            .build();
        Self { agent }
    }
}

#[derive(Debug)]
pub(crate) struct UreqError(String);

impl std::fmt::Display for UreqError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ureq transport error: {}", self.0)
    }
}

impl std::error::Error for UreqError {}

impl HttpTransport for UreqTransport {
    type Error = UreqError;

    fn send(
        &self,
        request: http::Request<Vec<u8>>,
    ) -> Result<http::Response<Vec<u8>>, Self::Error> {
        let (parts, body) = request.into_parts();
        let uri = parts.uri.to_string();
        let mut req = self.agent.request(parts.method.as_str(), &uri);
        for (name, value) in &parts.headers {
            // Cloud-metadata endpoints only accept ASCII header values
            // (tokens, IDs, fixed markers). Silently skip any header
            // whose value isn't printable ASCII — the request still
            // flows; the endpoint either accepts or rejects it on its
            // own terms.
            if let Ok(v) = value.to_str() {
                req = req.set(name.as_str(), v);
            }
        }

        let response = if body.is_empty() {
            req.call()
        } else {
            req.send_bytes(&body)
        };

        let ureq_resp = match response {
            // Non-2xx statuses are not transport errors per the library contract.
            Ok(r) | Err(ureq::Error::Status(_, r)) => r,
            Err(ureq::Error::Transport(t)) => return Err(UreqError(t.to_string())),
        };

        into_http_response(ureq_resp)
    }
}

fn into_http_response(resp: ureq::Response) -> Result<http::Response<Vec<u8>>, UreqError> {
    let mut builder = http::Response::builder().status(resp.status());
    for name in resp.headers_names() {
        if let Some(value) = resp.header(&name) {
            builder = builder.header(&name, value);
        }
    }
    let body = read_capped_body(resp)?;
    builder.body(body).map_err(|e| UreqError(e.to_string()))
}

fn read_capped_body(resp: ureq::Response) -> Result<Vec<u8>, UreqError> {
    let mut body = Vec::new();
    // Read one byte past the cap so we can detect overflow instead of
    // silently truncating.
    resp.into_reader()
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|e| UreqError(e.to_string()))?;
    if body.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(UreqError(format!(
            "response body exceeded {MAX_RESPONSE_BYTES} bytes"
        )));
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};
    use std::thread::{self, JoinHandle};
    use std::time::Instant;

    /// Upper bound on how long the mock server will wait for a client to
    /// connect and for socket reads/writes to progress. Prevents a test
    /// whose client never connects (or stalls mid-exchange) from hanging
    /// the suite — the server thread exits and the join in `Drop` unblocks.
    const MOCK_SERVER_DEADLINE: Duration = Duration::from_secs(10);

    /// One-shot mock server. Drops join the server thread so a hung
    /// server fails the test rather than leaking a zombie thread.
    struct MockServer {
        url: String,
        handle: Option<JoinHandle<()>>,
    }

    impl MockServer {
        fn serve_once(response: Vec<u8>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let addr = listener.local_addr().expect("local_addr");
            listener
                .set_nonblocking(true)
                .expect("set_nonblocking on listener");
            let handle = thread::spawn(move || {
                let deadline = Instant::now() + MOCK_SERVER_DEADLINE;
                let Some(mut stream) = accept_with_deadline(&listener, deadline) else {
                    return;
                };
                configure_server_stream(&mut stream);
                drain_request(&mut stream);
                let _ = stream.write_all(&response);
            });
            Self {
                url: format!("http://{addr}/"),
                handle: Some(handle),
            }
        }
    }

    fn accept_with_deadline(listener: &TcpListener, deadline: Instant) -> Option<TcpStream> {
        loop {
            match listener.accept() {
                Ok((stream, _)) => return Some(stream),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return None;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return None,
            }
        }
    }

    fn configure_server_stream(stream: &mut TcpStream) {
        // On BSD-derived platforms (macOS) the accepted socket inherits
        // O_NONBLOCK from the listener; on Linux it does not. Force
        // blocking so set_{read,write}_timeout actually govern the
        // exchange and ureq sees a normal stream.
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(MOCK_SERVER_DEADLINE));
        let _ = stream.set_write_timeout(Some(MOCK_SERVER_DEADLINE));
    }

    impl Drop for MockServer {
        fn drop(&mut self) {
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn drain_request(stream: &mut TcpStream) {
        use std::io::Read;
        let mut buf = [0u8; 1024];
        // One small read is enough to consume a typical GET request line
        // for the purposes of these tests.
        let _ = stream.read(&mut buf);
    }

    fn get(url: &str) -> Result<http::Response<Vec<u8>>, UreqError> {
        let transport = UreqTransport::with_timeout(Duration::from_secs(5));
        let request = http::Request::builder()
            .method("GET")
            .uri(url)
            .body(Vec::new())
            .expect("request builder");
        transport.send(request)
    }

    #[test]
    fn returns_non_2xx_as_response_not_error() {
        let server = MockServer::serve_once(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nnot found".to_vec(),
        );
        let resp = get(&server.url).expect("404 must not be a transport error");
        assert_eq!(resp.status(), 404);
        assert_eq!(resp.body(), b"not found");
    }

    #[test]
    fn propagates_2xx_body_and_status() {
        let server = MockServer::serve_once(
            b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\n\r\nhello-hostid!".to_vec(),
        );
        let resp = get(&server.url).expect("200 must succeed");
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.body(), b"hello-hostid!");
    }

    #[test]
    fn rejects_oversize_response_body() {
        let cap = usize::try_from(MAX_RESPONSE_BYTES).expect("cap fits in usize on test targets");
        let big = vec![b'x'; cap + 16];
        let mut payload =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", big.len()).into_bytes();
        payload.extend_from_slice(&big);
        let server = MockServer::serve_once(payload);
        let err = get(&server.url).expect_err("oversize body must error");
        // The cap is enforced either by the explicit length check in
        // `into_http_response` ("exceeded") or — when the underlying stream
        // closes before the taker fills — by the read_to_end error itself
        // (e.g. ureq's "response body closed before all bytes were read",
        // observed on macOS when the server-side TCP buffer can't hold the
        // whole 1 MiB before the agent tears the connection down). Both
        // outcomes satisfy the "oversize body must not silently pass" contract.
        let msg = err.to_string();
        assert!(
            msg.contains("exceeded") || msg.contains("closed before"),
            "oversize body should surface as cap or premature-close error: {err}",
        );
    }

    #[test]
    fn transport_error_when_connection_refused() {
        // Bind and drop immediately so the port is closed before the request.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        drop(listener);
        let err =
            get(&format!("http://{addr}/")).expect_err("closed port must return a transport error");
        // Pin the failure to a connection-level refusal so a regression
        // that swaps this to a timeout or TLS error (which would mask a
        // reachability change) is caught. `ureq` surfaces the OS error
        // text directly in the transport error message.
        let msg = err.to_string().to_ascii_lowercase();
        assert!(
            msg.contains("refused") || msg.contains("connect"),
            "expected connection-refused-style transport error, got: {err}"
        );
    }
}
