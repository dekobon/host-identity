//! `host-identity` — command-line interface for the `host-identity` crate.
//! Binary was renamed from `hostid` to avoid colliding with coreutils
//! `hostid(1)`; see `crates/host-identity-cli/Cargo.toml` for the
//! `[[bin]]` name and the rationale.
//!
//! This crate also exposes a small library surface so build tooling
//! (the workspace `xtask` that generates man pages) can reuse the
//! exact `clap::Command` definition the binary ships with. End users
//! should depend on the [`host-identity`] library directly.
//!
//! [`host-identity`]: https://crates.io/crates/host-identity

use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow};
use clap::{Parser, Subcommand, ValueEnum};
use host_identity::ids::{resolver_from_ids, source_ids};
use host_identity::sources::{AppSpecific, FileOverride};
use host_identity::{
    HostId, ResolveOutcome, Resolver, Source, SourceKind, UnknownSourceError, Wrap,
};
use serde::Serialize;

/// Environment variable that, when set to a non-empty path, causes the
/// CLI to prepend a [`FileOverride`] at the front of the resolver
/// chain. Takes precedence over `HOST_IDENTITY`.
const HOST_IDENTITY_FILE_ENV: &str = "HOST_IDENTITY_FILE";

#[cfg(feature = "network")]
mod transport;

/// Crate version, re-exported so the workspace `xtask` can stamp the
/// man page footer with the CLI crate's version rather than its own.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const LONG_ABOUT: &str = "\
Resolve a stable, collision-resistant host UUID across platforms, container \
runtimes, cloud providers, and Kubernetes.

host-identity walks a platform-appropriate chain of identity sources (env override, \
/etc/machine-id, DMI, cloud metadata, Kubernetes pod UID, …) and returns the \
first one that produces a credible identifier. Cloned-VM sentinels, empty \
files, and systemd's literal `uninitialized` string are rejected rather than \
silently hashed into a shared ID.

Two environment variables pin identity explicitly when the automatic chain \
gets it wrong. HOST_IDENTITY_FILE names a file whose contents are used as \
the host identifier and takes precedence over every other source, including \
HOST_IDENTITY. HOST_IDENTITY supplies the identifier inline and is consulted \
next. Both work with the default chain and with explicit --sources.

By default the chain uses only local sources. Pass --network to pull in \
cloud-metadata and Kubernetes probes, which require an HTTP client and a \
binary built with the `network` feature.";

const EXAMPLES: &str = "\
EXAMPLES:
    Print the host UUID using the default local source chain:
        host-identity

    Include cloud-metadata and Kubernetes sources:
        host-identity resolve --network

    Build a custom chain from explicit source identifiers:
        host-identity resolve --sources env-override,machine-id,dmi

    Derive a per-app UUID that doesn't leak the raw machine key:
        host-identity resolve --app-id com.example.telemetry

    Emit machine-readable output:
        host-identity resolve --format json
        host-identity audit --format json

    Pin identity via environment override:
        HOST_IDENTITY=11111111-2222-3333-4444-555555555555 host-identity

    Pin identity via a file (takes precedence over HOST_IDENTITY):
        HOST_IDENTITY_FILE=/etc/host-identity host-identity

    List every source identifier compiled into this binary:
        host-identity sources
";

/// Top-level command-line interface for the `host-identity` binary.
#[derive(Parser)]
#[command(
    name = "host-identity",
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
    /// shorthand for `host-identity resolve ...`).
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

    /// How the raw identifier is turned into a UUID.
    #[arg(
        long,
        value_enum,
        default_value_t = WrapArg::V5,
        long_help = "\
How the raw identifier returned by the winning source is turned into a UUID.

  v5           UUID v5 (SHA-1) under this crate's private namespace (default).
               Deterministic: the same raw input always produces the same
               UUID. Rehashes the raw value even when the source already
               yields a UUID (DMI product_uuid, macOS IOPlatformUUID,
               Windows MachineGuid, SMBIOS), so two tools that share a raw
               source cannot emit colliding IDs unless they also share this
               crate's namespace.

  v3           UUID v3 (MD5) under the nil namespace. Use ONLY for interop
               with existing pipelines that already produced IDs this way —
               notably the legacy Go derivation `uuid.NewMD5(uuid.Nil, raw)`.
               Prefer v5 for new deployments; RFC 9562 recommends v5 over v3.

  passthrough  Parse the raw value directly as a UUID, with no hashing.
               Use when the source already yields a UUID string and you
               want that exact UUID to survive unchanged — e.g. to match
               an ID another tool on the same host already emits. Fails
               with an error when the raw value is not a parseable UUID
               (machine-id, container IDs, Kubernetes pod UIDs all
               qualify; arbitrary strings from HOST_IDENTITY do not).

Pick v5 unless you have a concrete interop requirement.",
    )]
    wrap: WrapArg,

    /// Comma-separated source identifiers to build a custom chain
    /// (see `host-identity sources`). Combine with `--network` to include
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

    /// Wrap every source with an HMAC-SHA256 per-app derivation keyed on
    /// the inner source value. Emits a per-app UUID; the inner raw value
    /// never leaves the process.
    #[arg(
        long,
        value_name = "APP_ID",
        long_help = "\
Wrap every source in the chain with an HMAC-SHA256 per-app derivation \
keyed on the inner source value. When set, the resolver emits a per-app \
UUID and the inner source's raw value never leaves the process.

APP_ID is a UTF-8 byte string — reverse-DNS identifiers like \
`com.example.telemetry` are idiomatic, but any stable bytes work. It is \
NOT secret: privacy comes from not leaking the inner raw value, not from \
APP_ID being hidden. The derived value is an identifier, not key material. \
Callers needing a non-UTF-8 APP_ID must use the library API.

Effect on the chain:
  * Every source is wrapped, including the HOST_IDENTITY env override,
    HOST_IDENTITY_FILE, cloud-metadata, and Kubernetes sources.
  * Source labels in `--format json` and `audit` output become
    `app-specific:<inner>` (e.g. `app-specific:machine-id`).

Interaction with --wrap:
  * v5 (default)     re-hashes the AppSpecific UUID under this crate's
                     private namespace — per-app-unique AND
                     namespace-separated from other tools that re-hash
                     the same AppSpecific output.
  * passthrough      round-trips the AppSpecific UUID unchanged — the
                     \"byte-exact AppSpecific\" mode.
  * v3               works, but v5 is preferred.

Wrapping a source whose raw value is already public (cloud instance IDs, \
Kubernetes pod UIDs readable via the API server) adds no privacy — the \
input was not secret to begin with. Use this flag when you need to keep \
a local machine key (machine-id, DMI, IoPlatformUuid, MachineGuid, \
hostid, SMBIOS) out of your telemetry."
    )]
    app_id: Option<String>,
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

#[derive(ValueEnum, Serialize, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
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
            eprintln!("host-identity: {:#}", err.into_inner());
            code
        }
    }
}

/// Write to stdout, collapsing `BrokenPipe` into a clean exit.
/// Without this, piping `host-identity audit | head` panics.
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
    validate_resolve_args(args)?;
    let wrap = Wrap::from(args.wrap);
    let base = base_resolver(args)?.with_wrap(wrap);
    let with_override = prepend_file_override(base);
    Ok(apply_app_specific(
        with_override,
        args.app_id.as_deref(),
        wrap,
    ))
}

fn validate_resolve_args(args: &ResolveArgs) -> Result<(), CliError> {
    if args.network_timeout_ms.is_some() && !args.network {
        return usage(anyhow!("`--network-timeout-ms` requires `--network`"));
    }
    if matches!(args.app_id.as_deref(), Some("")) {
        return usage(anyhow!("`--app-id` must not be empty"));
    }
    // A stray comma in `--sources foo,,bar` (or a leading/trailing
    // comma) lets clap's `value_delimiter` admit an empty token. Reject
    // it here with a message that names the flag — otherwise the empty
    // id reaches `resolver_from_ids` and surfaces as
    // `unknown source identifier: ``` (empty backticks).
    if args.sources.iter().any(String::is_empty) {
        return usage(anyhow!("`--sources` contains an empty identifier"));
    }
    Ok(())
}

fn base_resolver(args: &ResolveArgs) -> Result<Resolver, CliError> {
    match (args.sources.is_empty(), args.network) {
        (true, false) => Ok(Resolver::with_defaults()),
        (true, true) => network_defaults(args.network_timeout_ms).map_err(CliError::Usage),
        (false, false) => {
            resolver_from_ids(&args.sources).map_err(|e| CliError::Usage(map_unknown(e)))
        }
        (false, true) => resolver_from_ids_network(&args.sources, args.network_timeout_ms)
            .map_err(CliError::Usage),
    }
}

fn prepend_file_override(resolver: Resolver) -> Resolver {
    match host_identity_file_override() {
        Some(file) => resolver.prepend(file),
        None => resolver,
    }
}

fn apply_app_specific(resolver: Resolver, app_id: Option<&str>, wrap: Wrap) -> Resolver {
    let Some(app_id) = app_id else {
        return resolver;
    };
    let id_bytes = app_id.as_bytes();
    let wrapped: Vec<Box<dyn Source>> = resolver
        .into_boxed_sources()
        .into_iter()
        .map(|s| Box::new(AppSpecific::new(s, id_bytes)) as Box<dyn Source>)
        .collect();
    Resolver::new().with_boxed_sources(wrapped).with_wrap(wrap)
}

/// Read `HOST_IDENTITY_FILE` from the process environment and, if set
/// to a non-empty path, return a [`FileOverride`] for it. The override
/// is prepended by [`build_resolver`] so it outranks every other source,
/// matching the documented precedence in `LONG_ABOUT`.
fn host_identity_file_override() -> Option<FileOverride> {
    file_override_from_env_value(std::env::var_os(HOST_IDENTITY_FILE_ENV).as_deref())
}

/// Pure helper: construct a [`FileOverride`] from a raw env-var value.
/// Returns `None` when the value is absent or empty. A set-but-empty
/// value is treated the same as unset so a script clearing the
/// variable (`HOST_IDENTITY_FILE=`) disables the override rather than
/// silently turning into `FileOverride::new("")` (which would probe a
/// relative empty path).
fn file_override_from_env_value(value: Option<&OsStr>) -> Option<FileOverride> {
    let raw = value?;
    if raw.is_empty() {
        return None;
    }
    Some(FileOverride::new(PathBuf::from(raw)))
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
    print_host_id(&id, args.format, args.wrap).map_err(CliError::Runtime)
}

fn run_audit(args: &ResolveArgs) -> Result<(), CliError> {
    let resolver = build_resolver(args)?;
    let outcomes = resolver.resolve_all();
    let mut buf = Vec::new();
    render_audit(&mut buf, args, &outcomes).map_err(CliError::Runtime)?;
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

fn render_audit(
    buf: &mut Vec<u8>,
    args: &ResolveArgs,
    outcomes: &[ResolveOutcome],
) -> anyhow::Result<()> {
    match args.format {
        Format::Json => render_audit_json(buf, args.wrap, outcomes),
        Format::Plain => render_audit_plain(buf, outcomes),
        Format::Summary => render_audit_summary(buf, outcomes),
    }
}

fn render_audit_json(
    buf: &mut Vec<u8>,
    wrap: WrapArg,
    outcomes: &[ResolveOutcome],
) -> anyhow::Result<()> {
    let report = AuditReport {
        wrap,
        entries: outcomes.iter().map(AuditEntry::from).collect(),
    };
    serde_json::to_writer_pretty(&mut *buf, &report)?;
    buf.push(b'\n');
    Ok(())
}

fn render_audit_plain(buf: &mut Vec<u8>, outcomes: &[ResolveOutcome]) -> anyhow::Result<()> {
    for (i, outcome) in outcomes.iter().enumerate() {
        let kind = outcome.source();
        let tail = match outcome {
            ResolveOutcome::Found(id) => id.summary().to_string(),
            ResolveOutcome::Skipped(_) => "(skipped)".to_owned(),
            ResolveOutcome::Errored(_, err) => format!("ERROR {err}"),
        };
        writeln!(buf, "{i:>2}. {kind:<28} -> {tail}")?;
    }
    Ok(())
}

/// One compact line per outcome, mirroring `resolve --format summary`'s
/// `source:uuid` shape. `Skipped` and `Errored` outcomes emit
/// `source:skipped` / `source:ERROR <msg>`. Note: some source labels
/// themselves contain a colon (e.g. `AppSpecific` renders as
/// `app-specific:<inner>`), and error text may contain arbitrary
/// characters, so consumers that want to recover the uuid should
/// `rsplit_once(':')` — UUIDs never contain a colon.
fn render_audit_summary(buf: &mut Vec<u8>, outcomes: &[ResolveOutcome]) -> anyhow::Result<()> {
    for outcome in outcomes {
        match outcome {
            ResolveOutcome::Found(id) => writeln!(buf, "{}", id.summary())?,
            ResolveOutcome::Skipped(kind) => writeln!(buf, "{kind}:skipped")?,
            ResolveOutcome::Errored(kind, err) => writeln!(buf, "{kind}:ERROR {err}")?,
        }
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

fn print_host_id(id: &HostId, format: Format, wrap: WrapArg) -> Result<()> {
    let mut buf = Vec::new();
    match format {
        Format::Plain => writeln!(buf, "{id}")?,
        Format::Summary => writeln!(buf, "{}", id.summary())?,
        Format::Json => {
            let out = HostIdReport {
                wrap,
                host_id: HostIdJson {
                    uuid: id.as_uuid().to_string(),
                    source: id.source().as_str(),
                    in_container: id.in_container(),
                },
            };
            serde_json::to_writer_pretty(&mut buf, &out)?;
            buf.push(b'\n');
        }
    }
    write_and_flush(&buf)?;
    Ok(())
}

#[derive(Serialize)]
struct HostIdReport {
    wrap: WrapArg,
    host_id: HostIdJson,
}

#[derive(Serialize)]
struct HostIdJson {
    uuid: String,
    source: &'static str,
    in_container: bool,
}

#[derive(Serialize)]
struct AuditReport {
    wrap: WrapArg,
    entries: Vec<AuditEntry>,
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
        source_ids::LINUX_HOSTID,
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
    fn file_override_from_env_value_handles_absent_empty_and_set() {
        assert!(file_override_from_env_value(None).is_none());
        assert!(file_override_from_env_value(Some(OsStr::new(""))).is_none());
        let fo = file_override_from_env_value(Some(OsStr::new("/tmp/host-id")))
            .expect("non-empty value must yield a FileOverride");
        assert_eq!(fo.path(), std::path::Path::new("/tmp/host-id"));
    }

    #[test]
    fn host_id_json_schema_is_stable() {
        // Pins the `--format json` schema for `host-identity resolve`. Any field
        // rename or case change breaks downstream script parsers; this
        // snapshot catches that at test time.
        let sample = HostIdReport {
            wrap: WrapArg::V5,
            host_id: HostIdJson {
                uuid: "11111111-2222-3333-4444-555555555555".to_owned(),
                source: "machine-id",
                in_container: false,
            },
        };
        let json = serde_json::to_value(&sample).unwrap();
        let obj = json.as_object().unwrap();
        assert_eq!(obj.len(), 2);
        assert_eq!(obj["wrap"], "v5");
        let inner = obj["host_id"].as_object().unwrap();
        assert_eq!(inner.len(), 3);
        assert_eq!(inner["uuid"], "11111111-2222-3333-4444-555555555555");
        assert_eq!(inner["source"], "machine-id");
        assert_eq!(inner["in_container"], false);
    }

    #[test]
    fn wrap_arg_serializes_to_lowercase_flag_string() {
        // The `wrap` field in JSON output must match the CLI flag values
        // verbatim so saved output round-trips back through `--wrap`.
        for (variant, expected) in [
            (WrapArg::V5, "v5"),
            (WrapArg::V3, "v3"),
            (WrapArg::Passthrough, "passthrough"),
        ] {
            assert_eq!(serde_json::to_value(variant).unwrap(), expected);
        }
    }

    #[test]
    fn audit_entry_schema_is_stable_for_every_status() {
        let outcomes = mixed_outcomes();
        let report = AuditReport {
            wrap: WrapArg::V5,
            entries: outcomes.iter().map(AuditEntry::from).collect(),
        };
        let json = serde_json::to_value(&report).unwrap();
        let envelope = json.as_object().unwrap();
        assert_eq!(envelope.len(), 2);
        assert_eq!(envelope["wrap"], "v5");
        let arr = envelope["entries"].as_array().unwrap();
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

    #[test]
    fn build_resolver_with_app_id_wraps_every_source() {
        let args = ResolveArgs {
            sources: vec!["env-override".into(), "machine-id".into()],
            app_id: Some("com.example.a".into()),
            ..Default::default()
        };
        let resolver = build_resolver(&args).expect("app-id build");
        let kinds = resolver.source_kinds();
        assert_eq!(kinds.len(), 2);
        for kind in &kinds {
            let label = kind.as_str();
            assert!(
                label.starts_with("app-specific:"),
                "expected wrapped label, got {label:?}",
            );
        }
    }

    #[test]
    fn build_resolver_with_empty_app_id_errors_usage() {
        let args = ResolveArgs {
            app_id: Some(String::new()),
            ..Default::default()
        };
        let err = build_resolver(&args).expect_err("empty app-id must fail");
        assert!(matches!(err, CliError::Usage(_)));
        assert!(err.into_inner().to_string().contains("must not be empty"));
    }

    #[test]
    fn validate_resolve_args_rejects_timeout_without_network() {
        let args = ResolveArgs {
            network_timeout_ms: Some(500),
            network: false,
            ..Default::default()
        };
        let err = validate_resolve_args(&args).expect_err("timeout without network must fail");
        assert!(matches!(err, CliError::Usage(_)));
        assert!(
            err.into_inner()
                .to_string()
                .contains("`--network-timeout-ms` requires `--network`")
        );
    }

    #[test]
    fn validate_resolve_args_accepts_timeout_with_network() {
        let args = ResolveArgs {
            network_timeout_ms: Some(500),
            network: true,
            ..Default::default()
        };
        validate_resolve_args(&args).expect("timeout with network must validate");
    }

    #[test]
    fn validate_resolve_args_accepts_default() {
        validate_resolve_args(&ResolveArgs::default()).expect("default args must validate");
    }

    #[test]
    fn validate_resolve_args_rejects_empty_source_identifier_in_every_position() {
        // Regression for #21. A stray comma anywhere in `--sources`
        // produces an empty token; previously this surfaced downstream
        // as `unknown source identifier: ``` with empty backticks.
        let cases: &[&[&str]] = &[
            &[""],                      // `--sources ""`
            &["", "machine-id"],        // `--sources ,machine-id`
            &["machine-id", ""],        // `--sources machine-id,`
            &["machine-id", "", "dmi"], // `--sources machine-id,,dmi`
            &["", ""],                  // `--sources ,`
        ];
        for ids in cases {
            let args = ResolveArgs {
                sources: ids.iter().map(|&s| s.to_string()).collect(),
                ..Default::default()
            };
            let Err(CliError::Usage(err)) = validate_resolve_args(&args) else {
                panic!("empty id {ids:?} must fail as a usage error");
            };
            let msg = err.to_string();
            assert!(
                msg.contains("`--sources`") && msg.contains("empty identifier"),
                "error should name the flag and describe the problem for {ids:?}: {msg}",
            );
        }
    }

    #[test]
    fn clap_parser_emits_empty_token_that_validation_catches() {
        // Guard against a future refactor of the `#[arg(... value_delimiter = ',')]`
        // attribute silently changing parse behaviour so empty tokens no
        // longer reach `validate_resolve_args`. If that ever happens this
        // test fails loudly instead of the validator becoming dead code.
        let cli = Cli::try_parse_from(["host-identity", "resolve", "--sources", "machine-id,,dmi"])
            .expect("clap must parse a doubled-comma source list");
        let Some(Command::Resolve(resolve)) = cli.command else {
            panic!("expected Resolve subcommand");
        };
        assert_eq!(
            resolve.sources,
            vec!["machine-id".to_owned(), String::new(), "dmi".to_owned()],
        );
        let Err(CliError::Usage(_)) = validate_resolve_args(&resolve) else {
            panic!("empty id must fail as a usage error");
        };
    }

    #[test]
    fn validate_resolve_args_accepts_non_empty_app_id() {
        let args = ResolveArgs {
            app_id: Some("com.example.telemetry".into()),
            ..Default::default()
        };
        validate_resolve_args(&args).expect("non-empty app-id must validate");
    }

    #[test]
    fn apply_app_specific_none_is_identity() {
        let resolver = Resolver::new()
            .push(host_identity::sources::EnvOverride::new("A"))
            .push(host_identity::sources::EnvOverride::new("B"))
            .with_wrap(Wrap::Passthrough);
        let before = resolver.source_kinds();
        let after = apply_app_specific(resolver, None, Wrap::Passthrough).source_kinds();
        assert_eq!(before, after);
        for kind in after {
            assert!(
                !kind.as_str().starts_with("app-specific:"),
                "None app-id must not wrap; got {kind:?}",
            );
        }
    }

    /// Shared fixture for the audit render tests: a three-source chain
    /// that yields exactly one `Found`, one `Errored`, and one
    /// `Skipped` outcome, in that order.
    fn mixed_outcomes() -> Vec<ResolveOutcome> {
        use host_identity::sources::FnSource;
        let found_src = FnSource::new(SourceKind::custom("ok"), || Ok(Some("raw".into())));
        let err_src = FnSource::new(SourceKind::custom("bad"), || {
            Err(host_identity::Error::Platform {
                source_kind: SourceKind::custom("bad"),
                reason: "synthetic".into(),
            })
        });
        let skip_src = FnSource::new(SourceKind::custom("skip"), || Ok(None));
        Resolver::new()
            .push(found_src)
            .push(err_src)
            .push(skip_src)
            .resolve_all()
    }

    #[test]
    fn render_audit_plain_formats_mixed_outcomes() {
        let outcomes = mixed_outcomes();
        let mut buf = Vec::new();
        render_audit_plain(&mut buf, &outcomes).expect("render");
        let text = String::from_utf8(buf).expect("utf-8");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        let arrow = lines[0].find(" -> ").expect("first line has arrow");
        for line in &lines {
            assert_eq!(
                line.find(" -> "),
                Some(arrow),
                "kind column should align across lines: {line:?}",
            );
        }
        assert!(lines[0].starts_with(" 0. ok "), "got: {:?}", lines[0]);
        assert!(lines[1].starts_with(" 1. bad "), "got: {:?}", lines[1]);
        assert!(lines[1].contains(" -> ERROR "));
        assert!(lines[1].contains("synthetic"));
        assert!(lines[2].starts_with(" 2. skip"), "got: {:?}", lines[2]);
        assert!(lines[2].ends_with(" -> (skipped)"), "got: {:?}", lines[2]);
    }

    #[test]
    fn render_audit_summary_produces_one_compact_line_per_outcome() {
        let outcomes = mixed_outcomes();
        let mut buf = Vec::new();
        render_audit_summary(&mut buf, &outcomes).expect("render");
        let text = String::from_utf8(buf).expect("utf-8");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(
            lines[0].starts_with("ok:"),
            "found line should lead with source:uuid, got: {:?}",
            lines[0]
        );
        let uuid_tail = lines[0].strip_prefix("ok:").expect("ok: prefix");
        assert_eq!(uuid_tail.len(), 36, "uuid tail: {uuid_tail:?}");
        // `Error::Platform` renders as `{source_kind}: {reason}`, so the
        // source label appears twice — once as the line's leading column
        // and once inside the error text. Matches `render_audit_plain`'s
        // `ERROR {err}` tail.
        assert_eq!(lines[1], "bad:ERROR bad: synthetic");
        assert_eq!(lines[2], "skip:skipped");
    }

    #[test]
    fn render_audit_summary_differs_from_plain() {
        let outcomes = mixed_outcomes();
        let mut plain = Vec::new();
        let mut summary = Vec::new();
        render_audit_plain(&mut plain, &outcomes).expect("plain");
        render_audit_summary(&mut summary, &outcomes).expect("summary");
        assert_ne!(
            plain, summary,
            "audit plain and summary must not collapse to identical output",
        );
    }
}
