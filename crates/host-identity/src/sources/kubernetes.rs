//! Kubernetes pod-level identity.
//!
//! Reads the pod UID from `/proc/self/mountinfo`, which contains the pod's
//! cgroup path in every CRI-compliant runtime. The UID there is assigned by
//! the kubelet and is stable for the lifetime of the pod — distinct from the
//! container ID used by [`crate::sources::ContainerId`].
//!
//! This source is file-based; it does not talk to the Kubernetes API server
//! and needs no HTTP client.
//!
//! Authoritative references:
//!
//! - [Kubernetes: Downward API volume files](https://kubernetes.io/docs/tasks/inject-data-application/downward-api-volume-expose-pod-information/)
//!   — `fieldRef: metadata.uid` projects the pod UID into a volume file;
//!   [`KubernetesDownwardApi`] consumes such files.
//! - [Kubernetes: Configure service accounts for pods](https://kubernetes.io/docs/tasks/configure-pod-container/configure-service-account/)
//!   — the kubelet auto-mounts `token`, `ca.crt`, and `namespace` under
//!   `/var/run/secrets/kubernetes.io/serviceaccount/`.
//!   [`KubernetesServiceAccount`] reads the `namespace` file.
//! - [Kubernetes: Identify container by pod UID and container name](https://kubernetes.io/docs/reference/instrumentation/cri-pod-container-metrics/)
//!   and the kubelet [cgroup manager
//!   documentation](https://kubernetes.io/docs/concepts/architecture/cgroups/)
//!   — document the cgroup-path shapes (`/kubepods/.../pod<uid>/...` and
//!   `kubepods-pod<uid>.slice` under the systemd cgroup driver) that
//!   [`KubernetesPodUid`] parses.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::source::{Probe, Source, SourceKind};
use crate::sources::util::{normalize, read_capped};

const DEFAULT_MOUNTINFO_PATH: &str = "/proc/self/mountinfo";

/// Upper bound on bytes read from mountinfo. Caps the streaming line
/// buffer so a pathological procfs cannot drive unbounded allocation.
const MAX_MOUNTINFO_BYTES: u64 = 2 * 1024 * 1024;
const DEFAULT_SA_NAMESPACE_PATH: &str = "/var/run/secrets/kubernetes.io/serviceaccount/namespace";

/// Kubernetes pod UID extracted from the pod's cgroup path.
#[derive(Debug, Clone)]
pub struct KubernetesPodUid {
    mountinfo_path: PathBuf,
}

impl KubernetesPodUid {
    /// Read from the standard `/proc/self/mountinfo` path.
    #[must_use]
    pub fn new() -> Self {
        Self {
            mountinfo_path: PathBuf::from(DEFAULT_MOUNTINFO_PATH),
        }
    }

    /// Read from a caller-supplied mountinfo path (tests, alternate procfs).
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            mountinfo_path: path.into(),
        }
    }
}

impl Default for KubernetesPodUid {
    fn default() -> Self {
        Self::new()
    }
}

impl Source for KubernetesPodUid {
    fn kind(&self) -> SourceKind {
        SourceKind::KubernetesPodUid
    }

    fn probe(&self) -> Result<Option<Probe>, Error> {
        Ok(extract_pod_uid(&self.mountinfo_path)
            .map(|uid| Probe::new(SourceKind::KubernetesPodUid, uid)))
    }
}

fn extract_pod_uid(path: &Path) -> Option<String> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            log::debug!(
                "host-identity: kubernetes-pod-uid: reading {}: {err}",
                path.display()
            );
            return None;
        }
    };
    BufReader::new(file.take(MAX_MOUNTINFO_BYTES))
        .lines()
        .map_while(Result::ok)
        .find_map(|line| line.split_ascii_whitespace().find_map(pod_uid_from_word))
}

/// A pod UID appears after the literal `pod` marker in the cgroup path, in
/// one of two forms the kubelet writes:
///
/// - `/kubepods/pod<uid>/…` (cgroup v1, dashed UUID)
/// - `/kubepods.slice/kubepods-pod<uid>.slice/…` (systemd cgroup driver,
///   underscores instead of dashes)
///
/// The UID is a canonical UUID — 36 chars with dashes or 36 chars with the
/// dashes rewritten as underscores. Accept either, lowercase-normalise
/// every hex byte, and normalise underscore-dashes to the canonical form.
///
/// The `pod` marker must be preceded by either start-of-word, `/`, or `-`
/// so that an embedded occurrence inside `kubepods` cannot false-match on
/// adjacent UUID-shaped bytes.
fn pod_uid_from_word(word: &str) -> Option<String> {
    let bytes = word.as_bytes();
    bytes
        .windows(3)
        .enumerate()
        .filter(|(start, marker)| *marker == b"pod" && is_pod_marker_boundary(bytes, *start))
        .find_map(|(start, _)| uid_after_marker(bytes.get(start + 3..)?))
}

fn uid_after_marker(rest: &[u8]) -> Option<String> {
    let candidate: &[u8; 36] = rest.get(..36)?.try_into().ok()?;
    if !has_consistent_separators(candidate) {
        return None;
    }
    let normalised = normalise_uuid_bytes(candidate);
    if !is_canonical_uuid_shape(&normalised) {
        return None;
    }
    let uid = std::str::from_utf8(&normalised).expect("ascii after normalisation");
    Some(uid.to_owned())
}

// Require a consistent separator style at the canonical UUID positions —
// either all `-` (dashed form) or all `_` (systemd form). A mix means the
// input is malformed; reject before normalisation so the shape check
// can't silently accept it.
fn has_consistent_separators(candidate: &[u8; 36]) -> bool {
    let seps = [candidate[8], candidate[13], candidate[18], candidate[23]];
    seps.iter().all(|b| *b == b'-') || seps.iter().all(|b| *b == b'_')
}

fn normalise_uuid_bytes(candidate: &[u8; 36]) -> [u8; 36] {
    let mut out = [0u8; 36];
    for (dst, src) in out.iter_mut().zip(candidate.iter()) {
        *dst = match *src {
            b'_' => b'-',
            c => c.to_ascii_lowercase(),
        };
    }
    out
}

fn is_pod_marker_boundary(bytes: &[u8], marker_start: usize) -> bool {
    let Some(prev_idx) = marker_start.checked_sub(1) else {
        return true;
    };
    matches!(bytes.get(prev_idx), Some(b'/' | b'-'))
}

fn is_canonical_uuid_shape(bytes: &[u8]) -> bool {
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => *b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

/// Kubernetes service-account namespace, read from the pod's in-mount
/// secret directory.
///
/// Yields the namespace string (e.g. `kube-system`). Coarse — every pod in
/// the namespace shares the same value — so use this as a fallback below
/// [`KubernetesPodUid`] rather than a primary identity source.
#[derive(Debug, Clone)]
pub struct KubernetesServiceAccount {
    path: PathBuf,
}

impl KubernetesServiceAccount {
    /// Read from the standard service-account mount path.
    #[must_use]
    pub fn new() -> Self {
        Self {
            path: PathBuf::from(DEFAULT_SA_NAMESPACE_PATH),
        }
    }

    /// Read from a caller-supplied path.
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl Default for KubernetesServiceAccount {
    fn default() -> Self {
        Self::new()
    }
}

impl Source for KubernetesServiceAccount {
    fn kind(&self) -> SourceKind {
        SourceKind::KubernetesServiceAccount
    }

    fn probe(&self) -> Result<Option<Probe>, Error> {
        read_identifier_file(&self.path, SourceKind::KubernetesServiceAccount)
    }
}

/// Kubernetes downward API file-projected identity.
///
/// Point this at whatever path the pod spec writes to — commonly
/// `/etc/podinfo/uid` or `/etc/podinfo/name` from a `downwardAPI` volume.
/// The source is a thin wrapper over file reads; by default it labels
/// probes `SourceKind::KubernetesDownwardApi`. Callers who register more
/// than one downward-API source (e.g. one for pod UID, one for pod name)
/// should use [`KubernetesDownwardApi::with_label`] to distinguish them.
///
/// # When to use this vs [`crate::sources::FileOverride`]
///
/// Both types read a single-line file and emit its contents. Pick
/// `KubernetesDownwardApi` when the file is projected by a Kubernetes
/// `downwardAPI` volume — the provenance label makes logs and
/// telemetry attribute the identity to the pod spec rather than to an
/// arbitrary operator config. Pick `FileOverride` for anything else:
/// a sysadmin-managed `/etc/my-app/host-id`, a TPM-sealed file, a
/// cron-generated secret. The file-read contract is identical; only
/// the [`SourceKind`] on resulting probes differs.
#[derive(Debug, Clone)]
pub struct KubernetesDownwardApi {
    path: PathBuf,
    kind: SourceKind,
}

impl KubernetesDownwardApi {
    /// Read from the given downward-API path, labelled as
    /// [`SourceKind::KubernetesDownwardApi`].
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: SourceKind::KubernetesDownwardApi,
        }
    }

    /// Read from the given downward-API path, labelled with the
    /// caller-supplied static string as [`SourceKind::Custom`].
    ///
    /// Use when registering multiple downward-API sources so their
    /// provenance in logs and on [`crate::HostId::source`] stays
    /// distinguishable.
    #[must_use]
    pub fn with_label(path: impl Into<PathBuf>, label: &'static str) -> Self {
        Self {
            path: path.into(),
            kind: SourceKind::Custom(label),
        }
    }
}

impl Source for KubernetesDownwardApi {
    fn kind(&self) -> SourceKind {
        self.kind
    }

    fn probe(&self) -> Result<Option<Probe>, Error> {
        read_identifier_file(&self.path, self.kind)
    }
}

fn read_identifier_file(path: &Path, kind: SourceKind) -> Result<Option<Probe>, Error> {
    match read_capped(path) {
        Ok(content) => Ok(normalize(&content).map(|v| Probe::new(kind, v))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Io {
            source_kind: kind,
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const UID: &str = "aa97c1e4-1bd2-11ee-be56-0242ac120002";

    #[test]
    fn cgroup_v1_path_yields_uid() {
        let word = format!("/kubepods/burstable/pod{UID}/cafebabe/rootfs");
        assert_eq!(pod_uid_from_word(&word).as_deref(), Some(UID));
    }

    #[test]
    fn systemd_cgroup_path_with_underscores_is_normalised() {
        let underscored = UID.replace('-', "_");
        let word = format!(
            "/kubepods.slice/kubepods-burstable.slice/kubepods-burstable-pod{underscored}.slice/"
        );
        assert_eq!(pod_uid_from_word(&word).as_deref(), Some(UID));
    }

    #[test]
    fn word_without_pod_marker_returns_none() {
        let word = "/some/random/path/with-a-dashed-uuid-aa97c1e4-1bd2-11ee-be56-0242ac120002";
        assert_eq!(pod_uid_from_word(word), None);
    }

    #[test]
    fn pod_marker_followed_by_non_uuid_returns_none() {
        assert_eq!(
            pod_uid_from_word("/kubepods/podNOT-A-UUID-JUST-GARBAGE-HERE-XXXX"),
            None
        );
    }

    #[test]
    fn extract_pod_uid_reads_mountinfo_file() {
        let line = format!(
            "42 41 0:52 / /sys/fs/cgroup ro,nosuid,nodev,noexec,relatime shared:20 - cgroup cgroup rw,seclabel,memory,cpuacct,cpuset,name=systemd,cgroup=/kubepods.slice/kubepods-pod{UID}.slice\n"
        );
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(line.as_bytes()).unwrap();
        assert_eq!(extract_pod_uid(f.path()).as_deref(), Some(UID));
    }

    #[test]
    fn extract_pod_uid_empty_file_is_none() {
        let f = tempfile::NamedTempFile::new().unwrap();
        assert_eq!(extract_pod_uid(f.path()), None);
    }

    #[test]
    fn is_canonical_uuid_shape_rejects_wrong_length() {
        assert!(!is_canonical_uuid_shape(b"too-short"));
        assert!(!is_canonical_uuid_shape(
            b"aa97c1e4-1bd2-11ee-be56-0242ac12000"
        ));
    }

    #[test]
    fn is_canonical_uuid_shape_rejects_wrong_dash_positions() {
        // Dashes shifted — first dash at position 7 instead of 8.
        assert!(!is_canonical_uuid_shape(
            b"aa97c1e-41bd2-11ee-be56-0242ac120002X"
        ));
    }

    #[test]
    fn service_account_reads_namespace_file() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, "kube-system").unwrap();
        let source = KubernetesServiceAccount::at(f.path());
        let probe = source.probe().unwrap().unwrap();
        assert_eq!(probe.kind(), SourceKind::KubernetesServiceAccount);
        assert_eq!(probe.value(), "kube-system");
    }

    #[test]
    fn service_account_missing_path_is_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let source = KubernetesServiceAccount::at(dir.path().join("namespace"));
        assert!(source.probe().unwrap().is_none());
    }

    #[test]
    fn service_account_empty_file_is_none() {
        let f = tempfile::NamedTempFile::new().unwrap();
        let source = KubernetesServiceAccount::at(f.path());
        assert!(source.probe().unwrap().is_none());
    }

    #[test]
    fn downward_api_reads_projected_file() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "{UID}").unwrap();
        let source = KubernetesDownwardApi::new(f.path());
        let probe = source.probe().unwrap().unwrap();
        assert_eq!(probe.kind(), SourceKind::KubernetesDownwardApi);
        assert_eq!(probe.value(), UID);
    }

    #[test]
    fn downward_api_missing_file_is_none() {
        let dir = tempfile::TempDir::new().unwrap();
        let source = KubernetesDownwardApi::new(dir.path().join("uid"));
        assert!(source.probe().unwrap().is_none());
    }

    #[test]
    fn downward_api_with_label_uses_custom_kind() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "my-pod").unwrap();
        let source = KubernetesDownwardApi::with_label(f.path(), "pod-name");
        let probe = source.probe().unwrap().unwrap();
        assert_eq!(probe.kind(), SourceKind::Custom("pod-name"));
        assert_eq!(probe.value(), "my-pod");
    }

    #[test]
    fn service_account_reports_io_error_for_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        let source = KubernetesServiceAccount::at(dir.path());
        match source.probe() {
            Err(Error::Io { path, .. }) => assert_eq!(path, dir.path()),
            other => panic!("expected Error::Io, got {other:?}"),
        }
    }

    #[test]
    fn downward_api_reports_io_error_for_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        let source = KubernetesDownwardApi::new(dir.path());
        match source.probe() {
            Err(Error::Io { path, .. }) => assert_eq!(path, dir.path()),
            other => panic!("expected Error::Io, got {other:?}"),
        }
    }

    #[test]
    fn pod_uid_rejected_when_preceding_byte_is_letter_or_digit() {
        // 36-char UUID-shaped run immediately after `kubepods` with no
        // separator — the pod-marker boundary check must reject both
        // letters and digits that would collapse into a longer word.
        let letter_prefixed = format!("xkubepods{UID}/rest");
        assert_eq!(pod_uid_from_word(&letter_prefixed), None);
        let digit_prefixed = format!("0kubepods{UID}/rest");
        assert_eq!(pod_uid_from_word(&digit_prefixed), None);
    }

    #[test]
    fn pod_uid_rejects_mixed_separator_style() {
        // Mixed `_` and `-` at canonical positions (8/13/18/23) must be
        // rejected — a prior version mapped `_` to `-` before the shape
        // check and silently accepted this malformed input.
        let mixed = "aa97c1e4_1bd2-11ee_be56-0242ac120002";
        let word = format!("/kubepods/pod{mixed}/rest");
        assert_eq!(pod_uid_from_word(&word), None);
    }

    #[test]
    fn pod_uid_lowercases_uppercase_input() {
        let upper = UID.to_ascii_uppercase();
        let word = format!("/kubepods/pod{upper}/rest");
        assert_eq!(pod_uid_from_word(&word).as_deref(), Some(UID));
    }
}
