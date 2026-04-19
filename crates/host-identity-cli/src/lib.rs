//! `hostid` — command-line interface for the `host-identity` crate.
//!
//! This crate also exposes a small library surface so build tooling
//! (the workspace `xtask` that generates man pages) can reuse the
//! exact `clap::Command` definition the binary ships with. End users
//! should depend on the [`host-identity`] library directly.
//!
//! [`host-identity`]: https://crates.io/crates/host-identity

use std::io::{self, Write};
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow};
use clap::{Parser, Subcommand, ValueEnum};
use host_identity::ids::{resolver_from_ids, source_ids};
use host_identity::{HostId, ResolveOutcome, Resolver, SourceKind, UnknownSourceError, Wrap};
use serde::Serialize;

#[cfg(feature = "network")]
mod transport;

/// Crate version, re-exported so the workspace `xtask` can stamp the
/// man page footer with the CLI crate's version rather than its own.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const LONG_ABOUT: &str = "\
Resolve a stable, collision-resistant host UUID across platforms, container \
runtimes, cloud providers, and Kubernetes.

hostid walks a platform-appropriate chain of identity sources (env override, \
/etc/machine-id, DMI, cloud metadata, Kubernetes pod UID, …) and returns the \
first one that produces a credible identifier. Cloned-VM sentinels, empty \
files, and systemd's literal `uninitialized` string are rejected rather than \
silently hashed into a shared ID.

The HOST_IDENTITY environment variable (or a file named by HOST_IDENTITY_FILE) \
takes precedence over every other source, so operators can pin identity \
explicitly when the automatic chain gets it wrong.

By default the chain uses only local sources. Pass --network to pull in \
cloud-metadata and Kubernetes probes, which require an HTTP client and a \
binary built with the `network` feature.";

const EXAMPLES: &str = "\
EXAMPLES:
    Print the host UUID using the default local source chain:
        hostid

    Include cloud-metadata and Kubernetes sources:
        hostid resolve --network

    Build a custom chain from explicit source identifiers:
        hostid resolve --sources env-override,machine-id,dmi

    Emit machine-readable output:
        hostid resolve --format json
        hostid audit --format json

    Pin identity via environment override:
        HOST_IDENTITY=11111111-2222-3333-4444-555555555555 hostid

    List every source identifier compiled into this binary:
        hostid sources
";

/// Top-level command-line interface for the `hostid` binary.
#[derive(Parser)]
#[command(
    name = "hostid",
    version,
    author,
    about = "Resolve a stable host UUID across platforms, clouds, and Kubernetes",
    long_about = LONG_ABOUT,
    after_long_help = EXAMPLES,
    args_conflicts_with_subcommands = true,
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Top-level flags apply only when no subcommand is given (they are
    /// shorthand for `hostid resolve ...`).
    #[command(flatten)]
    resolve: ResolveArgs,
}

#[derive(Subcommand)]
enum Command {
    /// Resolve the host identity and print it (default).
    Resolve(ResolveArgs),
    /// Walk every source without short-circuiting and report each outcome.
    Audit(AuditArgs),
    /// List every source identifier compiled into this binary.
    Sources {
        /// Emit JSON instead of one identifier per line.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Parser, Clone, Default)]
struct ResolveArgs {
    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Plain)]
    format: Format,

    /// UUID wrap strategy.
    #[arg(long, value_enum, default_value_t = WrapArg::V5)]
    wrap: WrapArg,

    /// Comma-separated source identifiers to build a custom chain
    /// (see `hostid sources`). Combine with `--network` to include
    /// cloud-metadata sources in the chain.
    #[arg(long, value_delimiter = ',')]
    sources: Vec<String>,

    /// Enable cloud-metadata and Kubernetes sources by supplying an HTTP
    /// transport. Without `--sources` this adds them to the default chain;
    /// with `--sources` it lets identifiers like `aws-imds` resolve.
    /// Requires the binary to be built with the `network` feature.
    #[arg(long)]
    network: bool,

    /// Per-request timeout, in milliseconds, for cloud-metadata and
    /// Kubernetes HTTP probes. Only meaningful with `--network`. Off-cloud
    /// hosts never answer these endpoints, so this directly bounds the
    /// time spent waiting before falling through to the next source.
    #[arg(long, value_name = "MS", value_parser = clap::value_parser!(u64).range(1..))]
    network_timeout_ms: Option<u64>,
}

#[derive(Parser, Clone, Default)]
struct AuditArgs {
    #[command(flatten)]
    resolve: ResolveArgs,
}

#[derive(ValueEnum, Clone, Copy, Default)]
enum Format {
    #[default]
    Plain,
    Summary,
    Json,
}

#[derive(ValueEnum, Clone, Copy, Default)]
enum WrapArg {
    #[default]
    V5,
    V3,
    Passthrough,
}

impl From<WrapArg> for Wrap {
    fn from(w: WrapArg) -> Self {
        match w {
            WrapArg::V5 => Wrap::UuidV5Namespaced,
            WrapArg::V3 => Wrap::UuidV3Nil,
            WrapArg::Passthrough => Wrap::Passthrough,
        }
    }
}

/// Exit codes surfaced by the CLI. Scripts can branch on
/// `Usage` (2) vs. `Runtime` (1) to distinguish a bad invocation
/// from a host where no source produced an identity.
const EXIT_USAGE: u8 = 2;

/// Errors that `build_resolver` converts into an `EXIT_USAGE` exit.
#[derive(Debug)]
enum CliError {
    Usage(anyhow::Error),
    Runtime(anyhow::Error),
}

impl CliError {
    fn exit_code(&self) -> ExitCode {
        match self {
            Self::Usage(_) => ExitCode::from(EXIT_USAGE),
            Self::Runtime(_) => ExitCode::FAILURE,
        }
    }
    fn into_inner(self) -> anyhow::Error {
        match self {
            Self::Usage(e) | Self::Runtime(e) => e,
        }
    }
}

fn usage<T>(msg: anyhow::Error) -> Result<T, CliError> {
    Err(CliError::Usage(msg))
}

fn runtime_err<E: Into<anyhow::Error>>(e: E) -> CliError {
    CliError::Runtime(e.into())
}

fn runtime<T>(msg: anyhow::Error) -> Result<T, CliError> {
    Err(CliError::Runtime(msg))
}

/// Parse argv and run the CLI, returning the process exit code.
#[must_use]
pub fn run() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Resolve(args)) => run_resolve(&args),
        Some(Command::Audit(args)) => run_audit(&args.resolve),
        Some(Command::Sources { json }) => run_sources(json),
        None => run_resolve(&cli.resolve),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let code = err.exit_code();
            eprintln!("hostid: {:#}", err.into_inner());
            code
        }
    }
}

/// Write to stdout, collapsing `BrokenPipe` into a clean exit.
/// Without this, piping `hostid audit | head` panics.
fn write_and_flush(bytes: &[u8]) -> io::Result<()> {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    match lock.write_all(bytes).and_then(|()| lock.flush()) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(err) => Err(err),
    }
}

fn build_resolver(args: &ResolveArgs) -> Result<Resolver, CliError> {
    if args.network_timeout_ms.is_some() && !args.network {
        return usage(anyhow!("`--network-timeout-ms` requires `--network`"));
    }
    let resolver = match (args.sources.is_empty(), args.network) {
        (true, false) => Resolver::with_defaults(),
        (true, true) => network_defaults(args.network_timeout_ms).map_err(CliError::Usage)?,
        (false, false) => {
            resolver_from_ids(&args.sources).map_err(|e| CliError::Usage(map_unknown(e)))?
        }
        (false, true) => resolver_from_ids_network(&args.sources, args.network_timeout_ms)
            .map_err(CliError::Usage)?,
    };

    Ok(resolver.with_wrap(Wrap::from(args.wrap)))
}

#[cfg(feature = "network")]
#[allow(clippy::unnecessary_wraps)]
fn network_defaults(timeout_ms: Option<u64>) -> Result<Resolver> {
    Ok(Resolver::with_network_defaults(build_transport(timeout_ms)))
}

#[cfg(not(feature = "network"))]
fn network_defaults(_timeout_ms: Option<u64>) -> Result<Resolver> {
    Err(network_feature_disabled())
}

#[cfg(feature = "network")]
fn resolver_from_ids_network(ids: &[String], timeout_ms: Option<u64>) -> Result<Resolver> {
    host_identity::ids::resolver_from_ids_with_transport(ids, build_transport(timeout_ms))
        .map_err(map_unknown)
}

#[cfg(not(feature = "network"))]
fn resolver_from_ids_network(_ids: &[String], _timeout_ms: Option<u64>) -> Result<Resolver> {
    Err(network_feature_disabled())
}

#[cfg(feature = "network")]
fn build_transport(timeout_ms: Option<u64>) -> transport::UreqTransport {
    let timeout = timeout_ms.map_or(
        transport::DEFAULT_NETWORK_TIMEOUT,
        std::time::Duration::from_millis,
    );
    transport::UreqTransport::with_timeout(timeout)
}

#[cfg(not(feature = "network"))]
fn network_feature_disabled() -> anyhow::Error {
    anyhow!("this build has no `network` feature; rebuild with `--features network`")
}

fn map_unknown(err: UnknownSourceError) -> anyhow::Error {
    match err {
        UnknownSourceError::Unknown(id) => anyhow!("unknown source identifier: `{id}`"),
        UnknownSourceError::RequiresPath(id) => anyhow!(
            "source `{id}` requires a caller-supplied path and cannot be built from an identifier",
        ),
        UnknownSourceError::RequiresTransport(id) => {
            anyhow!("source `{id}` is a cloud source; pass `--network` to supply an HTTP transport")
        }
        UnknownSourceError::FeatureDisabled(id, feat) => anyhow!(
            "source `{id}` requires the `{feat}` feature, which isn't enabled in this build",
        ),
    }
}

fn run_resolve(args: &ResolveArgs) -> Result<(), CliError> {
    let resolver = build_resolver(args)?;
    let id = resolver
        .resolve()
        .context("no source produced a host identity")
        .map_err(CliError::Runtime)?;
    print_host_id(&id, args.format).map_err(CliError::Runtime)
}

fn run_audit(args: &ResolveArgs) -> Result<(), CliError> {
    let resolver = build_resolver(args)?;
    let outcomes = resolver.resolve_all();
    let mut buf = Vec::new();
    match args.format {
        Format::Json => {
            let report: Vec<AuditEntry> = outcomes.iter().map(AuditEntry::from).collect();
            serde_json::to_writer_pretty(&mut buf, &report).map_err(runtime_err)?;
            buf.push(b'\n');
        }
        Format::Plain | Format::Summary => {
            for (i, outcome) in outcomes.iter().enumerate() {
                let kind = outcome.source();
                let tail = match outcome {
                    ResolveOutcome::Found(id) => id.summary().to_string(),
                    ResolveOutcome::Skipped(_) => "(skipped)".to_owned(),
                    ResolveOutcome::Errored(_, err) => format!("ERROR {err}"),
                };
                writeln!(buf, "{i:>2}. {kind:<28} -> {tail}").map_err(runtime_err)?;
            }
        }
    }
    write_and_flush(&buf).map_err(runtime_err)?;

    // Exit non-zero (runtime) when every outcome errored or skipped —
    // nothing to show for the walk, matching `run_resolve`'s contract.
    if !outcomes
        .iter()
        .any(|o| matches!(o, ResolveOutcome::Found(_)))
    {
        return runtime(anyhow!("no source produced a host identity"));
    }
    Ok(())
}

fn run_sources(json: bool) -> Result<(), CliError> {
    let ids = available_source_ids();
    let mut buf = Vec::new();
    if json {
        let entries: Vec<SourceEntry> = ids
            .iter()
            .map(|id| SourceEntry {
                id,
                description: describe_id(id),
            })
            .collect();
        serde_json::to_writer_pretty(&mut buf, &entries).map_err(runtime_err)?;
        buf.push(b'\n');
    } else {
        // Source identifiers are ASCII; char count == byte count. Use
        // `chars().count()` anyway so a future non-ASCII label doesn't
        // silently desync the padding width.
        let width = ids
            .iter()
            .map(|id| id.chars().count())
            .max()
            .unwrap_or_default();
        for id in &ids {
            writeln!(buf, "{id:<width$}  {}", describe_id(id), width = width)
                .map_err(runtime_err)?;
        }
    }
    write_and_flush(&buf).map_err(runtime_err)
}

fn describe_id(id: &str) -> &'static str {
    SourceKind::from_id(id).map_or("", SourceKind::describe)
}

#[derive(Serialize)]
struct SourceEntry {
    id: &'static str,
    description: &'static str,
}

fn print_host_id(id: &HostId, format: Format) -> Result<()> {
    let mut buf = Vec::new();
    match format {
        Format::Plain => writeln!(buf, "{id}")?,
        Format::Summary => writeln!(buf, "{}", id.summary())?,
        Format::Json => {
            let out = HostIdJson {
                uuid: id.as_uuid().to_string(),
                source: id.source().as_str(),
                in_container: id.in_container(),
            };
            serde_json::to_writer_pretty(&mut buf, &out)?;
            buf.push(b'\n');
        }
    }
    write_and_flush(&buf)?;
    Ok(())
}

#[derive(Serialize)]
struct HostIdJson {
    uuid: String,
    source: &'static str,
    in_container: bool,
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum AuditStatus {
    Found,
    Skipped,
    Errored,
}

#[derive(Serialize)]
struct AuditEntry {
    source: &'static str,
    status: AuditStatus,
    uuid: Option<String>,
    error: Option<String>,
    in_container: Option<bool>,
}

impl From<&ResolveOutcome> for AuditEntry {
    fn from(o: &ResolveOutcome) -> Self {
        let source = o.source().as_str();
        match o {
            ResolveOutcome::Found(id) => Self {
                source,
                status: AuditStatus::Found,
                uuid: Some(id.as_uuid().to_string()),
                error: None,
                in_container: Some(id.in_container()),
            },
            ResolveOutcome::Skipped(_) => Self {
                source,
                status: AuditStatus::Skipped,
                uuid: None,
                error: None,
                in_container: None,
            },
            ResolveOutcome::Errored(_, err) => Self {
                source,
                status: AuditStatus::Errored,
                uuid: None,
                error: Some(err.to_string()),
                in_container: None,
            },
        }
    }
}

fn available_source_ids() -> Vec<&'static str> {
    let mut ids = vec![
        source_ids::ENV_OVERRIDE,
        source_ids::FILE_OVERRIDE,
        source_ids::MACHINE_ID,
        source_ids::DBUS_MACHINE_ID,
        source_ids::DMI,
        source_ids::IO_PLATFORM_UUID,
        source_ids::WINDOWS_MACHINE_GUID,
        source_ids::FREEBSD_HOSTID,
        source_ids::KENV_SMBIOS,
        source_ids::BSD_KERN_HOSTID,
        source_ids::ILLUMOS_HOSTID,
    ];
    #[cfg(feature = "container")]
    {
        ids.push(source_ids::CONTAINER);
        ids.push(source_ids::LXC);
    }
    #[cfg(feature = "network")]
    {
        ids.extend_from_slice(&[
            source_ids::AWS_IMDS,
            source_ids::GCP_METADATA,
            source_ids::AZURE_IMDS,
            source_ids::DIGITAL_OCEAN_METADATA,
            source_ids::HETZNER_METADATA,
            source_ids::OCI_METADATA,
            source_ids::KUBERNETES_POD_UID,
            source_ids::KUBERNETES_SERVICE_ACCOUNT,
            source_ids::KUBERNETES_DOWNWARD_API,
        ]);
    }
    ids.sort_unstable();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_arg_maps_every_variant_to_library_wrap() {
        assert!(matches!(Wrap::from(WrapArg::V5), Wrap::UuidV5Namespaced));
        assert!(matches!(Wrap::from(WrapArg::V3), Wrap::UuidV3Nil));
        assert!(matches!(
            Wrap::from(WrapArg::Passthrough),
            Wrap::Passthrough
        ));
    }

    #[test]
    fn available_source_ids_is_sorted_and_deduplicated() {
        let ids = available_source_ids();
        assert!(
            ids.windows(2).all(|w| w[0] < w[1]),
            "ids must be strictly sorted"
        );
        assert!(ids.contains(&source_ids::MACHINE_ID));
        assert!(ids.contains(&source_ids::DMI));
    }

    #[test]
    #[cfg(feature = "container")]
    fn available_source_ids_includes_container_when_feature_enabled() {
        assert!(available_source_ids().contains(&source_ids::CONTAINER));
        assert!(available_source_ids().contains(&source_ids::LXC));
    }

    #[test]
    fn build_resolver_defaults_when_no_flags_given() {
        let args = ResolveArgs::default();
        let resolver = build_resolver(&args).expect("defaults build");
        assert!(
            resolver
                .source_kinds()
                .contains(&host_identity::SourceKind::EnvOverride),
            "default chain must include env-override",
        );
    }

    #[test]
    fn build_resolver_uses_ids_chain_when_sources_set() {
        let args = ResolveArgs {
            sources: vec!["env-override".into(), "machine-id".into()],
            ..Default::default()
        };
        let resolver = build_resolver(&args).expect("ids build");
        let kinds = resolver.source_kinds();
        assert_eq!(kinds.len(), 2);
        assert_eq!(kinds[0], host_identity::SourceKind::EnvOverride);
        assert_eq!(kinds[1], host_identity::SourceKind::MachineId);
    }

    #[test]
    fn build_resolver_rejects_unknown_source_id() {
        let args = ResolveArgs {
            sources: vec!["definitely-not-a-source".into()],
            ..Default::default()
        };
        let err = build_resolver(&args).expect_err("unknown id must fail");
        assert!(
            err.into_inner()
                .to_string()
                .contains("unknown source identifier")
        );
    }

    #[test]
    #[cfg(feature = "network")]
    fn build_resolver_network_defaults_includes_cloud_sources() {
        let args = ResolveArgs {
            network: true,
            ..Default::default()
        };
        let resolver = build_resolver(&args).expect("network defaults build");
        assert!(
            resolver
                .source_kinds()
                .contains(&host_identity::SourceKind::AwsImds),
            "--network should add cloud sources to the default chain",
        );
    }

    #[test]
    #[cfg(feature = "network")]
    fn build_resolver_network_plus_ids_resolves_cloud_identifiers() {
        let args = ResolveArgs {
            sources: vec!["aws-imds".into()],
            network: true,
            ..Default::default()
        };
        let resolver = build_resolver(&args).expect("network + ids build");
        assert_eq!(
            resolver.source_kinds(),
            vec![host_identity::SourceKind::AwsImds]
        );
    }

    #[test]
    #[cfg(not(feature = "network"))]
    fn build_resolver_network_without_feature_errors() {
        let args = ResolveArgs {
            network: true,
            ..Default::default()
        };
        let err = build_resolver(&args).expect_err("--network must fail without feature");
        assert!(err.into_inner().to_string().contains("`network` feature"));
    }

    #[test]
    fn build_resolver_rejects_network_timeout_without_network() {
        let args = ResolveArgs {
            network_timeout_ms: Some(500),
            ..Default::default()
        };
        let err = build_resolver(&args).expect_err("must reject timeout without --network");
        assert!(
            err.into_inner()
                .to_string()
                .contains("requires `--network`")
        );
    }

    #[test]
    fn map_unknown_formats_each_variant_distinctly() {
        let cases = [
            (
                UnknownSourceError::Unknown("weird".to_owned()),
                "unknown source identifier",
            ),
            (
                UnknownSourceError::RequiresPath("file-override"),
                "caller-supplied path",
            ),
            (
                UnknownSourceError::RequiresTransport("aws-imds"),
                "pass `--network`",
            ),
            (
                UnknownSourceError::FeatureDisabled("aws-imds", "aws"),
                "isn't enabled in this build",
            ),
        ];
        for (err, expected_fragment) in cases {
            let msg = map_unknown(err).to_string();
            assert!(
                msg.contains(expected_fragment),
                "message {msg:?} missing fragment {expected_fragment:?}",
            );
        }
    }

    #[test]
    fn host_id_json_schema_is_stable() {
        // Pins the `--format json` schema for `hostid resolve`. Any field
        // rename or case change breaks downstream script parsers; this
        // snapshot catches that at test time.
        let sample = HostIdJson {
            uuid: "11111111-2222-3333-4444-555555555555".to_owned(),
            source: "machine-id",
            in_container: false,
        };
        let json = serde_json::to_value(&sample).unwrap();
        let obj = json.as_object().unwrap();
        assert_eq!(obj.len(), 3);
        assert_eq!(obj["uuid"], "11111111-2222-3333-4444-555555555555");
        assert_eq!(obj["source"], "machine-id");
        assert_eq!(obj["in_container"], false);
    }

    #[test]
    fn audit_entry_schema_is_stable_for_every_status() {
        use host_identity::sources::FnSource;
        let found_src = FnSource::new(SourceKind::custom("ok"), || Ok(Some("raw".into())));
        let err_src = FnSource::new(SourceKind::custom("bad"), || {
            Err(host_identity::Error::Platform {
                source_kind: SourceKind::custom("bad"),
                reason: "synthetic".into(),
            })
        });
        let skip_src = FnSource::new(SourceKind::custom("skip"), || Ok(None));
        let outcomes = Resolver::new()
            .push(found_src)
            .push(err_src)
            .push(skip_src)
            .resolve_all();
        let entries: Vec<AuditEntry> = outcomes.iter().map(AuditEntry::from).collect();
        let json = serde_json::to_value(&entries).unwrap();
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["status"], "found");
        assert!(arr[0]["uuid"].is_string());
        assert_eq!(arr[0]["error"], serde_json::Value::Null);
        assert_eq!(arr[1]["status"], "errored");
        assert!(arr[1]["error"].as_str().unwrap().contains("synthetic"));
        assert_eq!(arr[1]["uuid"], serde_json::Value::Null);
        assert_eq!(arr[2]["status"], "skipped");
        // Every entry shares the same key set.
        for entry in arr {
            let keys: Vec<_> = entry.as_object().unwrap().keys().collect();
            assert_eq!(keys.len(), 5);
        }
    }

    #[test]
    #[cfg(feature = "network")]
    fn available_source_ids_includes_every_cloud_and_k8s_source() {
        let ids = available_source_ids();
        for id in [
            source_ids::AWS_IMDS,
            source_ids::GCP_METADATA,
            source_ids::AZURE_IMDS,
            source_ids::DIGITAL_OCEAN_METADATA,
            source_ids::HETZNER_METADATA,
            source_ids::OCI_METADATA,
            source_ids::KUBERNETES_POD_UID,
            source_ids::KUBERNETES_SERVICE_ACCOUNT,
            source_ids::KUBERNETES_DOWNWARD_API,
        ] {
            assert!(ids.contains(&id), "missing {id}");
        }
    }
}
