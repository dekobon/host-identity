//! Integration tests exercising the public API.

use std::io::Write;

use host_identity::sources::{EnvOverride, FileOverride, FnSource};
use host_identity::{Error, ResolveOutcome, Resolver, Source, SourceKind, Wrap};
use serial_test::serial;
use tempfile::NamedTempFile;

/// Scope-bound env-var setter that removes the variable on drop.
///
/// Ensures a panic mid-test cannot leak the mutation into the next
/// `#[serial]` test — `unsafe { remove_var }` runs even on unwind.
struct EnvGuard {
    key: &'static str,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        // SAFETY: test-only env-var mutation; `#[serial]` prevents races.
        unsafe { std::env::set_var(key, value) };
        Self { key }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: test-only env-var mutation; restore happens on every
        // exit path including unwinding panics.
        unsafe { std::env::remove_var(self.key) };
    }
}

#[test]
#[serial]
fn env_override_resolves() {
    let var = "HOST_IDENTITY_TEST_ENV";
    let _guard = EnvGuard::set(var, "my-fleet-host-42");

    let id = Resolver::new()
        .push(EnvOverride::new(var))
        .resolve()
        .unwrap();
    assert_eq!(id.source(), SourceKind::EnvOverride);
    assert_eq!(
        id.as_uuid(),
        Wrap::UuidV5Namespaced.apply("my-fleet-host-42").unwrap()
    );
}

#[test]
fn file_override_resolves() {
    let mut f = NamedTempFile::new().unwrap();
    writeln!(f, "file-supplied-id").unwrap();

    let id = Resolver::new()
        .push(FileOverride::new(f.path()))
        .resolve()
        .unwrap();
    assert_eq!(id.source(), SourceKind::FileOverride);
    // The raw file value must be what got wrapped — not a placeholder and
    // not the wrong source's raw value.
    assert_eq!(
        id.as_uuid(),
        Wrap::UuidV5Namespaced.apply("file-supplied-id").unwrap()
    );
}

#[test]
fn custom_fn_source_resolves() {
    let custom = FnSource::new(SourceKind::custom("test-imds"), || Ok(Some("abc".into())));
    let id = Resolver::new().push(custom).resolve().unwrap();
    assert_eq!(id.source(), SourceKind::Custom("test-imds"));
    assert_eq!(id.as_uuid(), Wrap::UuidV5Namespaced.apply("abc").unwrap());
}

#[test]
#[serial]
fn chain_order_is_respected() {
    let var = "HOST_IDENTITY_TEST_ORDER";
    let _guard = EnvGuard::set(var, "from-env");

    let id = Resolver::new()
        .push(FnSource::new(SourceKind::custom("first"), || {
            Ok(Some("from-fn".into()))
        }))
        .push(EnvOverride::new(var))
        .resolve()
        .unwrap();
    assert_eq!(id.source(), SourceKind::Custom("first"));
}

#[test]
fn prepend_overrides_default_chain() {
    // Prepending a successful source should win regardless of what the
    // platform defaults would have returned.
    let id = Resolver::with_defaults()
        .prepend(FnSource::new(SourceKind::custom("forced"), || {
            Ok(Some("forced-value".into()))
        }))
        .resolve()
        .unwrap();
    assert_eq!(id.source(), SourceKind::Custom("forced"));
    assert_eq!(
        id.as_uuid(),
        Wrap::UuidV5Namespaced.apply("forced-value").unwrap()
    );
}

#[test]
fn empty_chain_reports_no_source() {
    let err = Resolver::new()
        .resolve()
        .expect_err("empty chain must fail");
    match err {
        Error::NoSource { tried } => assert!(
            tried.is_empty(),
            "empty chain must report empty `tried`, got {tried:?}"
        ),
        other => panic!("expected Error::NoSource, got {other:?}"),
    }
}

#[test]
fn no_source_lists_every_tried_kind() {
    let err = Resolver::new()
        .push(FnSource::new(SourceKind::custom("a"), || Ok(None)))
        .push(FnSource::new(SourceKind::custom("b"), || Ok(None)))
        .resolve()
        .expect_err("chain of Ok(None) must fail");
    match err {
        Error::NoSource { tried } => assert_eq!(tried, "a,b"),
        other => panic!("expected Error::NoSource, got {other:?}"),
    }
}

#[test]
fn passthrough_wrap_rejects_non_uuid() {
    let src = FnSource::new(SourceKind::custom("bad-uuid"), || {
        Ok(Some("not-a-uuid".into()))
    });
    let err = Resolver::new()
        .push(src)
        .with_wrap(Wrap::Passthrough)
        .resolve()
        .expect_err("non-uuid passthrough must fail");
    assert!(matches!(err, Error::Malformed { .. }));
}

#[test]
fn passthrough_wrap_accepts_uuid() {
    let uuid_str = "12345678-1234-1234-1234-123456789abc";
    let src = FnSource::new(SourceKind::custom("ok-uuid"), move || {
        Ok(Some(uuid_str.into()))
    });
    let id = Resolver::new()
        .push(src)
        .with_wrap(Wrap::Passthrough)
        .resolve()
        .unwrap();
    assert_eq!(id.to_string(), uuid_str);
}

#[cfg(feature = "aws")]
#[test]
#[serial]
fn network_defaults_uses_transport_for_cloud_sources() {
    use host_identity::transport::HttpTransport;
    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};

    // A fake transport that serves canned IMDSv2 responses. Cloneable
    // because `Resolver::with_network_defaults` requires `Clone` so each
    // cloud source gets its own handle.
    #[derive(Clone)]
    struct FakeImds(Arc<Mutex<std::collections::VecDeque<http::Response<Vec<u8>>>>>);

    impl HttpTransport for FakeImds {
        type Error = Infallible;
        fn send(
            &self,
            _request: http::Request<Vec<u8>>,
        ) -> Result<http::Response<Vec<u8>>, Self::Error> {
            Ok(self.0.lock().unwrap().pop_front().unwrap_or_else(|| {
                // Unexpected additional requests short-circuit as 404 so
                // the source returns Ok(None) rather than panicking — keeps
                // this test robust to future cloud-feature additions.
                http::Response::builder()
                    .status(404)
                    .body(Vec::new())
                    .unwrap()
            }))
        }
    }

    let iid = r#"{"instanceId": "i-network-defaults"}"#;
    let transport = FakeImds(Arc::new(Mutex::new(
        [
            http::Response::builder()
                .status(200)
                .body(b"tok".to_vec())
                .unwrap(),
            http::Response::builder()
                .status(200)
                .body(iid.as_bytes().to_vec())
                .unwrap(),
        ]
        .into(),
    )));

    // Clear any ambient HOST_IDENTITY so the env override doesn't win.
    unsafe { std::env::remove_var("HOST_IDENTITY") };

    let id = Resolver::with_network_defaults(transport)
        .resolve()
        .unwrap();
    assert_eq!(id.source(), SourceKind::AwsImds);
    assert_eq!(
        id.as_uuid(),
        Wrap::UuidV5Namespaced.apply("i-network-defaults").unwrap()
    );
}

#[cfg(feature = "aws")]
#[test]
#[serial]
fn network_defaults_env_override_wins_over_cloud() {
    use host_identity::transport::HttpTransport;
    use std::convert::Infallible;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct NeverCalled;
    impl HttpTransport for NeverCalled {
        type Error = Infallible;
        fn send(
            &self,
            _request: http::Request<Vec<u8>>,
        ) -> Result<http::Response<Vec<u8>>, Self::Error> {
            panic!("env override should short-circuit before any transport call");
        }
    }
    // Silence the unused-import warnings when only one cloud feature is on.
    let _ = (Arc::new(Mutex::new(0u8)),);

    let var = "HOST_IDENTITY";
    let _guard = EnvGuard::set(var, "env-wins-over-cloud");

    let id = Resolver::with_network_defaults(NeverCalled)
        .resolve()
        .unwrap();

    assert_eq!(id.source(), SourceKind::EnvOverride);
    assert_eq!(
        id.as_uuid(),
        Wrap::UuidV5Namespaced.apply("env-wins-over-cloud").unwrap()
    );
}

#[test]
fn resolve_all_walks_every_source_in_chain_order() {
    // A chain that mixes a success, an Ok(None), and an error. The
    // short-circuiting resolve() would stop at the first Err; resolve_all
    // must surface every outcome.
    let failing = FnSource::new(SourceKind::custom("fails"), || {
        Err(Error::Platform {
            source_kind: SourceKind::custom("fails"),
            reason: "synthetic".to_owned(),
        })
    });
    let missing = FnSource::new(SourceKind::custom("missing"), || Ok(None));
    let ok = FnSource::new(SourceKind::custom("ok"), || Ok(Some("raw-x".into())));

    let outcomes = Resolver::new()
        .push(failing)
        .push(missing)
        .push(ok)
        .resolve_all();

    assert_eq!(outcomes.len(), 3);

    match &outcomes[0] {
        ResolveOutcome::Errored(
            kind,
            Error::Platform {
                source_kind,
                reason,
            },
        ) => {
            assert_eq!(*kind, SourceKind::Custom("fails"));
            assert_eq!(*source_kind, SourceKind::Custom("fails"));
            assert_eq!(reason, "synthetic");
        }
        other => panic!("expected Errored(fails, Platform), got {other:?}"),
    }
    match &outcomes[1] {
        ResolveOutcome::Skipped(kind) => assert_eq!(*kind, SourceKind::Custom("missing")),
        other => panic!("expected Skipped(missing), got {other:?}"),
    }
    match &outcomes[2] {
        ResolveOutcome::Found(id) => {
            assert_eq!(id.source(), SourceKind::Custom("ok"));
            assert_eq!(id.as_uuid(), Wrap::UuidV5Namespaced.apply("raw-x").unwrap());
        }
        other => panic!("expected Found(ok), got {other:?}"),
    }
}

#[test]
fn resolve_all_reports_wrap_failure_as_errored_not_found() {
    // Passthrough wrap on a non-UUID value: resolve() would return
    // Err(Malformed); resolve_all captures it as an Errored outcome on
    // that specific source without aborting.
    let bad = FnSource::new(SourceKind::custom("not-uuid"), || Ok(Some("nope".into())));
    let good_uuid = "12345678-1234-1234-1234-123456789abc";
    let good = FnSource::new(SourceKind::custom("is-uuid"), move || {
        Ok(Some(good_uuid.into()))
    });

    let outcomes = Resolver::new()
        .push(bad)
        .push(good)
        .with_wrap(Wrap::Passthrough)
        .resolve_all();

    assert_eq!(outcomes.len(), 2);
    match &outcomes[0] {
        ResolveOutcome::Errored(kind, Error::Malformed { reason, .. }) => {
            assert_eq!(*kind, SourceKind::Custom("not-uuid"));
            assert!(reason.contains("nope"));
        }
        other => panic!("expected Errored(not-uuid, Malformed), got {other:?}"),
    }
    assert!(matches!(&outcomes[1], ResolveOutcome::Found(_)));
}

#[test]
fn resolve_all_with_caller_chosen_subset() {
    // The same builder that feeds `resolve()` feeds `resolve_all()`, so
    // specifying an exact subset is just `Resolver::new().push(...)`.
    let mut f = NamedTempFile::new().unwrap();
    writeln!(f, "from-file").unwrap();

    let outcomes = Resolver::new()
        .push(FileOverride::new(f.path()))
        .push(FnSource::new(SourceKind::custom("extra"), || Ok(None)))
        .resolve_all();

    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].source(), SourceKind::FileOverride);
    assert_eq!(outcomes[1].source(), SourceKind::Custom("extra"));
    assert!(outcomes[0].host_id().is_some());
    assert!(outcomes[1].host_id().is_none());
    assert_eq!(
        outcomes[0].host_id().unwrap().as_uuid(),
        Wrap::UuidV5Namespaced.apply("from-file").unwrap(),
    );
}

#[test]
fn free_function_resolve_all_returns_one_outcome_per_default_source() {
    // Which sources *succeed* on this host is environment-dependent, but
    // `resolve_all` is documented to return one outcome per chain source,
    // in chain order — assert that invariant so a regression that drops or
    // reorders a source is caught.
    let expected_kinds: Vec<SourceKind> = Resolver::with_defaults().source_kinds();
    let outcomes = host_identity::resolve_all();
    let actual_kinds: Vec<SourceKind> = outcomes.iter().map(ResolveOutcome::source).collect();
    assert_eq!(actual_kinds, expected_kinds);
}

#[test]
fn with_boxed_sources_accepts_heterogeneous_chain() {
    // Different concrete types — EnvOverride vs FnSource — which
    // with_sources cannot accept. with_boxed_sources takes them.
    let chain: Vec<Box<dyn Source>> = vec![
        Box::new(EnvOverride::new("HOST_IDENTITY_NEVER_SET")),
        Box::new(FnSource::new(SourceKind::custom("fallback"), || {
            Ok(Some("fallback-raw".into()))
        })),
    ];

    let id = Resolver::new().with_boxed_sources(chain).resolve().unwrap();
    assert_eq!(id.source(), SourceKind::Custom("fallback"));
    assert_eq!(
        id.as_uuid(),
        Wrap::UuidV5Namespaced.apply("fallback-raw").unwrap()
    );
}

#[test]
fn source_kinds_iter_matches_vec_form() {
    let resolver = Resolver::new()
        .push(EnvOverride::new("X"))
        .push(FnSource::new(SourceKind::custom("y"), || Ok(None)));
    let via_vec = resolver.source_kinds();
    let via_iter: Vec<SourceKind> = resolver.source_kinds_iter().collect();
    assert_eq!(via_vec, via_iter);
}

#[test]
fn uninitialized_error_propagates_through_resolver() {
    // Regression test for the crate's headline contract: a source that
    // reports `Error::Uninitialized` must surface through `resolve()`
    // unchanged, so callers can distinguish the systemd early-boot
    // window from "no source produced a value".
    let src = FnSource::new(SourceKind::custom("machine-id-like"), || {
        Err(Error::Uninitialized {
            source_kind: SourceKind::custom("machine-id-like"),
            path: "/etc/machine-id".into(),
        })
    });
    let err = Resolver::new()
        .push(src)
        .resolve()
        .expect_err("uninitialized must not be silently skipped");
    assert!(matches!(err, Error::Uninitialized { .. }));
    assert!(!err.is_recoverable());
}

/// Custom source that bypasses `FnSource`'s built-in `normalize()` —
/// lets tests simulate an ill-behaved `Source` impl that forgets to
/// trim or reject empty strings. The resolver's central guard must
/// catch it.
#[derive(Debug)]
struct RawSource(&'static str);
impl Source for RawSource {
    fn kind(&self) -> SourceKind {
        SourceKind::custom("raw")
    }
    fn probe(&self) -> Result<Option<host_identity::Probe>, Error> {
        Ok(Some(host_identity::Probe::new(self.kind(), self.0)))
    }
}

#[test]
fn empty_raw_identifier_is_rejected_as_malformed() {
    // A custom `Source` that returns an empty string must not produce a
    // valid identity — otherwise every such source hashes to the same
    // deterministic "empty" UUID.
    let err = Resolver::new()
        .push(RawSource(""))
        .resolve()
        .expect_err("empty raw must fail");
    assert!(
        matches!(&err, Error::Malformed { reason, .. } if reason.contains("empty")),
        "expected empty-raw Malformed, got {err:?}"
    );
}

#[test]
fn empty_raw_identifier_in_resolve_all_becomes_errored() {
    let outcomes = Resolver::new().push(RawSource("   ")).resolve_all();
    assert_eq!(outcomes.len(), 1);
    assert!(matches!(
        &outcomes[0],
        ResolveOutcome::Errored(_, Error::Malformed { .. })
    ));
}

#[test]
fn wrap_is_deterministic_across_calls() {
    let a = Resolver::new()
        .push(FnSource::new(SourceKind::custom("x"), || {
            Ok(Some("stable".into()))
        }))
        .resolve()
        .unwrap();
    let b = Resolver::new()
        .push(FnSource::new(SourceKind::custom("x"), || {
            Ok(Some("stable".into()))
        }))
        .resolve()
        .unwrap();
    assert_eq!(a.as_uuid(), b.as_uuid());
}
