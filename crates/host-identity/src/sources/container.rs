//! Container runtime identity.
//!
//! Extracts the container ID from `/proc/self/mountinfo`. Matches agent-go's
//! patterns so the extracted ID is wire-compatible with existing telemetry
//! pipelines:
//!
//! - `/docker/<64-hex>` (Docker)
//! - `:<64-hex>` (Kubernetes CRI via containerd)
//! - `/system.slice/crio-<64-hex>.scope` (CRI-O systemd scope units)
//! - `containers/<64-hex>` (Podman / CRI-O)
//! - `sandboxes/<64-hex>` (containerd)
//!
//! Authoritative references:
//!
//! - [OCI Runtime Specification](https://github.com/opencontainers/runtime-spec/blob/main/spec.md)
//!   — defines the container ID as an opaque but unique handle assigned by
//!   the runtime; the 64-char lowercase hex shape is a runtime-level
//!   convention (Docker / containerd / CRI-O) rather than a spec
//!   requirement.
//! - Linux [`proc_pid_mountinfo(5)`](https://man7.org/linux/man-pages/man5/proc_pid_mountinfo.5.html)
//!   — documents the per-process mountinfo format this source parses.
//! - [`cgroups(7)`](https://man7.org/linux/man-pages/man7/cgroups.7.html)
//!   — documents the cgroup hierarchy names that container runtimes embed
//!   in their mount paths (`/docker/...`, `kubepods-...`, `crio-...`).
//!
//! # Identity scope
//!
//! `ContainerId` is **per-container** scope. A process running on a
//! bare host (no container runtime) sees no match in `mountinfo` and
//! the source returns `Ok(None)` — the resolver falls through to the
//! host-scope sources beneath it. A process running in a container
//! returns the container's runtime-assigned ID, distinct from every
//! sibling container on the same host. Placing `ContainerId` above
//! the per-instance cloud sources and the per-host-OS sources is
//! what prevents every container on one host from colliding onto the
//! host's identity; the default chains do this for you. See
//! `docs/algorithm.md` → "Identity scope".

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::source::{Probe, Source, SourceKind};

const DEFAULT_MOUNTINFO_PATH: &str = "/proc/self/mountinfo";

/// Upper bound on bytes read from mountinfo. Production systems with
/// hundreds of mounts stay well under 1 MiB; capping at 2 MiB prevents
/// an adversarial or corrupt procfs from exhausting memory through the
/// internal line buffer of [`BufRead::lines`].
const MAX_MOUNTINFO_BYTES: u64 = 2 * 1024 * 1024;

/// Container ID extracted from a mountinfo file.
#[derive(Debug, Clone)]
pub struct ContainerId {
    mountinfo_path: PathBuf,
}

impl ContainerId {
    /// Read from the standard `/proc/self/mountinfo` path.
    #[must_use]
    pub fn new() -> Self {
        Self {
            mountinfo_path: PathBuf::from(DEFAULT_MOUNTINFO_PATH),
        }
    }

    /// Read from a caller-supplied mountinfo path (useful for tests and
    /// alternate procfs mount points).
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            mountinfo_path: path.into(),
        }
    }
}

impl Default for ContainerId {
    fn default() -> Self {
        Self::new()
    }
}

impl Source for ContainerId {
    fn kind(&self) -> SourceKind {
        SourceKind::Container
    }

    fn probe(&self) -> Result<Option<Probe>, Error> {
        Ok(extract_container_id(&self.mountinfo_path)
            .map(|id| Probe::new(SourceKind::Container, id)))
    }
}

fn extract_container_id(path: &Path) -> Option<String> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            log::debug!(
                "host-identity: container: reading {}: {err}",
                path.display()
            );
            return None;
        }
    };
    BufReader::new(file.take(MAX_MOUNTINFO_BYTES))
        .lines()
        .map_while(Result::ok)
        .find_map(|line| {
            line.split_ascii_whitespace()
                .find_map(container_id_from_word)
        })
}

/// Runtime tokens that must appear somewhere in a mountinfo word for its
/// 64-hex substring to be accepted as a container ID. Keeps incidental
/// `/<64hex>/` paths (e.g. overlay `lowerdir=/var/lib/foo/<64hex>/data`
/// from an unrelated tool) out of the match set.
const RUNTIME_TOKENS: &[&str] = &[
    "docker",
    "kubepods",
    "containerd",
    "crio",
    "containers",
    "libpod",
    "sandboxes",
];

fn word_has_runtime_token(word: &str) -> bool {
    RUNTIME_TOKENS.iter().any(|tok| word.contains(tok))
}

fn container_id_from_word(word: &str) -> Option<String> {
    if !word_has_runtime_token(word) {
        return None;
    }
    let bytes = word.as_bytes();
    bytes.windows(64).enumerate().find_map(|(start, run)| {
        if !is_hex_run(run) || !matches_surrounding(&bytes[..start], &bytes[start + 64..]) {
            return None;
        }
        let id = std::str::from_utf8(run).expect("ascii hex is valid utf-8");
        Some(id.to_owned())
    })
}

fn is_hex_run(bytes: &[u8]) -> bool {
    bytes.iter().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn matches_surrounding(prefix: &[u8], suffix: &[u8]) -> bool {
    // End-anchored: `/<hex>$` and `:<hex>$`.
    if suffix.is_empty() && matches!(prefix.last(), Some(b'/' | b':')) {
        return true;
    }
    // End-anchored: `/.+-<hex>.scope$` — a `/` must precede the trailing
    // `-` with at least one character between them.
    if suffix == b".scope" && prefix.last() == Some(&b'-') {
        let before_dash = &prefix[..prefix.len() - 1];
        if let Some(pos) = before_dash.iter().position(|&b| b == b'/') {
            if pos + 1 < before_dash.len() {
                return true;
            }
        }
    }
    // Un-anchored: `containers/<hex>` and `sandboxes/<hex>`.
    prefix.ends_with(b"containers/") || prefix.ends_with(b"sandboxes/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_pattern_matches() {
        let hex = "a".repeat(64);
        assert_eq!(container_id_from_word(&format!("/docker/{hex}")), Some(hex));
    }

    #[test]
    fn rejects_short_hex() {
        assert_eq!(container_id_from_word("/docker/abc"), None);
    }

    #[test]
    fn scope_pattern_rejects_non_hex_tail() {
        let tail = "z".repeat(64);
        assert_eq!(container_id_from_word(&format!("/crio-{tail}.scope")), None);
    }

    #[test]
    fn extract_container_id_reads_mountinfo_file() {
        use std::io::Write;
        let hex = "b".repeat(64);
        let line = format!(
            "1 2 0:0 / /host rw,relatime - overlay overlay rw,lowerdir=/var/lib/docker/containers/{hex}/hostname\n"
        );
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(line.as_bytes()).unwrap();
        assert_eq!(extract_container_id(f.path()), Some(hex));
    }

    #[test]
    fn extract_container_id_empty_file_is_none() {
        let f = tempfile::NamedTempFile::new().unwrap();
        assert_eq!(extract_container_id(f.path()), None);
    }

    #[test]
    fn colon_pattern_matches() {
        let hex = "c".repeat(64);
        assert_eq!(
            container_id_from_word(&format!("docker://sha256:{hex}")),
            Some(hex)
        );
    }

    #[test]
    fn scope_pattern_matches() {
        let hex = "d".repeat(64);
        assert_eq!(
            container_id_from_word(&format!("/system.slice/crio-{hex}.scope")),
            Some(hex)
        );
    }

    #[test]
    fn sandboxes_pattern_matches() {
        let hex = "e".repeat(64);
        assert_eq!(
            container_id_from_word(&format!("/run/containerd/sandboxes/{hex}/rootfs")),
            Some(hex)
        );
    }

    #[test]
    fn scope_pattern_requires_slash_before_dash() {
        let hex = "f".repeat(64);
        // No `/` anywhere before the trailing `-`.
        assert_eq!(container_id_from_word(&format!("crio-{hex}.scope")), None);
    }

    #[test]
    fn scope_pattern_requires_char_between_slash_and_dash() {
        let hex = "0".repeat(64);
        // `/` is immediately before `-` — `.+` in the original regex requires
        // at least one char between them.
        assert_eq!(container_id_from_word(&format!("/-{hex}.scope")), None);
    }

    #[test]
    fn bare_hex_without_delimiter_is_rejected() {
        let hex = "1".repeat(64);
        assert_eq!(container_id_from_word(&hex), None);
    }

    #[test]
    fn incidental_hex_path_without_runtime_token_is_rejected() {
        // Overlay `lowerdir=` from an unrelated tool — the 64-hex run sits
        // under `/<64hex>/` but the word carries no container-runtime token.
        let hex = "2".repeat(64);
        let word = format!("lowerdir=/var/lib/foo/{hex}/data");
        assert_eq!(container_id_from_word(&word), None);
    }
}
