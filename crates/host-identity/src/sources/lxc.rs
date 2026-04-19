//! LXC / LXD container identity.
//!
//! Extracts a user-chosen container name from `/proc/self/cgroup` or
//! `/proc/self/mountinfo` and composes it with `/etc/machine-id` into a
//! stable raw identifier of the shape `lxc:<machine_id>:<name>`. Names
//! alone are not unique across hosts — two unrelated hosts can run a
//! container called `web`. Salting with the host's `machine-id` makes
//! the composed identifier host-unique before the resolver's [`Wrap`]
//! stage hashes it into a UUID.
//!
//! Recognised markers (first match wins):
//!
//! - `/lxc.payload.<name>` — LXC ≥ 3.1 payload cgroup (modern LXC and LXD).
//! - `/lxc.monitor.<name>` — LXC ≥ 3.1 monitor cgroup (lxc-monitord).
//! - `/lxc/<name>`         — legacy pre-3.1 cgroup v1 layout.
//!
//! Any of the three may be wrapped in a systemd transient unit and so
//! end in `.scope` or `.service`; the parser strips one such suffix.
//!
//! # Identity scope
//!
//! Per-container, like [`super::ContainerId`]. In a nested deployment
//! (e.g. Docker inside an LXD container) the resolver's short-circuit
//! behaviour together with the default chain — `ContainerId` before
//! `LxcId` — ensures the innermost scope wins.
//!
//! [`Wrap`]: crate::wrap::Wrap

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::source::{Probe, Source, SourceKind};
use crate::sources::util::{MAX_ID_FILE_BYTES, normalize, read_capped};

const DEFAULT_CGROUP_PATH: &str = "/proc/self/cgroup";
const DEFAULT_MOUNTINFO_PATH: &str = "/proc/self/mountinfo";
const DEFAULT_MACHINE_ID_PATH: &str = "/etc/machine-id";

/// Upper bound on bytes read from `/proc/self/mountinfo`. Matches the
/// cap used by [`super::ContainerId`] for the same reason: bound memory
/// against a corrupt or adversarial procfs.
const MAX_MOUNTINFO_BYTES: u64 = 2 * 1024 * 1024;

/// LXC / LXD container name extractor.
#[derive(Debug, Clone)]
pub struct LxcId {
    cgroup: PathBuf,
    mountinfo: PathBuf,
    machine_id: PathBuf,
}

impl LxcId {
    /// Read from the standard procfs and `/etc/machine-id` paths.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cgroup: PathBuf::from(DEFAULT_CGROUP_PATH),
            mountinfo: PathBuf::from(DEFAULT_MOUNTINFO_PATH),
            machine_id: PathBuf::from(DEFAULT_MACHINE_ID_PATH),
        }
    }

    /// Override the cgroup path. Useful for tests and alternate
    /// procfs mount points.
    #[must_use]
    pub fn with_cgroup(mut self, path: impl Into<PathBuf>) -> Self {
        self.cgroup = path.into();
        self
    }

    /// Override the mountinfo path.
    #[must_use]
    pub fn with_mountinfo(mut self, path: impl Into<PathBuf>) -> Self {
        self.mountinfo = path.into();
        self
    }

    /// Override the machine-id path used as a salt.
    #[must_use]
    pub fn with_machine_id(mut self, path: impl Into<PathBuf>) -> Self {
        self.machine_id = path.into();
        self
    }
}

impl Default for LxcId {
    fn default() -> Self {
        Self::new()
    }
}

impl Source for LxcId {
    fn kind(&self) -> SourceKind {
        SourceKind::Lxc
    }

    fn probe(&self) -> Result<Option<Probe>, Error> {
        let Some(machine_id) = read_machine_id(&self.machine_id) else {
            return Ok(None);
        };
        let Some(name) = scan_file(&self.cgroup, MAX_ID_FILE_BYTES)
            .or_else(|| scan_file(&self.mountinfo, MAX_MOUNTINFO_BYTES))
        else {
            return Ok(None);
        };
        let raw = format!("lxc:{machine_id}:{name}");
        Ok(Some(Probe::new(SourceKind::Lxc, raw)))
    }
}

/// Read machine-id and return a usable trimmed value. `NotFound`,
/// empty, and the `uninitialized` sentinel all map to `None` — the
/// source should silently fall through rather than abort the chain
/// from a secondary position.
fn read_machine_id(path: &Path) -> Option<String> {
    let content = read_capped(path).ok()?;
    normalize(&content).map(str::to_owned)
}

fn scan_file(path: &Path, cap: u64) -> Option<String> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            log::debug!("host-identity: lxc: reading {}: {err}", path.display());
            return None;
        }
    };
    BufReader::new(file.take(cap))
        .lines()
        .map_while(Result::ok)
        .find_map(|line| {
            line.split_ascii_whitespace()
                .find_map(extract_name_from_word)
        })
}

fn extract_name_from_word(word: &str) -> Option<String> {
    word.split(':').find_map(match_lxc_marker)
}

fn match_lxc_marker(segment: &str) -> Option<String> {
    // `.payload.` / `.monitor.` with literal dots don't occur in
    // generic Linux paths, so substring match is safe. Legacy
    // `/lxc/` is prefix-only so `/usr/share/lxc/templates/…` and
    // `/var/lib/lxc-templates/…` don't false-match.
    [LXC_PAYLOAD, LXC_MONITOR]
        .iter()
        .find_map(|&m| extract_name(&segment[segment.find(m)? + m.len()..]))
        .or_else(|| segment.strip_prefix(LXC_LEGACY).and_then(extract_name))
}

const LXC_PAYLOAD: &str = "/lxc.payload.";
const LXC_MONITOR: &str = "/lxc.monitor.";
const LXC_LEGACY: &str = "/lxc/";

fn extract_name(rest: &str) -> Option<String> {
    let end = rest
        .bytes()
        .position(|b| !is_name_byte(b))
        .unwrap_or(rest.len());
    let candidate = &rest[..end];
    let candidate = candidate
        .strip_suffix(".scope")
        .or_else(|| candidate.strip_suffix(".service"))
        .unwrap_or(candidate);
    (!candidate.is_empty()).then(|| candidate.to_owned())
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn probe_with(cgroup: &str, mountinfo: &str, machine_id: &str) -> Option<Probe> {
        let mut cg = NamedTempFile::new().unwrap();
        cg.write_all(cgroup.as_bytes()).unwrap();
        let mut mi = NamedTempFile::new().unwrap();
        mi.write_all(mountinfo.as_bytes()).unwrap();
        let mut id = NamedTempFile::new().unwrap();
        id.write_all(machine_id.as_bytes()).unwrap();
        LxcId::new()
            .with_cgroup(cg.path())
            .with_mountinfo(mi.path())
            .with_machine_id(id.path())
            .probe()
            .unwrap()
    }

    #[test]
    fn payload_in_cgroup_v2_matches() {
        let probe = probe_with("0::/lxc.payload.demo\n", "", "abc123\n").unwrap();
        assert_eq!(probe.kind(), SourceKind::Lxc);
        assert_eq!(probe.value(), "lxc:abc123:demo");
    }

    #[test]
    fn payload_in_cgroup_v1_matches() {
        let cg = "4:memory:/lxc.payload.demo/init.scope\n";
        let probe = probe_with(cg, "", "abc123\n").unwrap();
        assert_eq!(probe.value(), "lxc:abc123:demo");
    }

    #[test]
    fn monitor_matches() {
        let cg = "0::/lxc.monitor.demo\n";
        let probe = probe_with(cg, "", "abc123\n").unwrap();
        assert_eq!(probe.value(), "lxc:abc123:demo");
    }

    #[test]
    fn legacy_slash_lxc_matches() {
        let cg = "11:name=systemd:/lxc/demo\n";
        let probe = probe_with(cg, "", "abc123\n").unwrap();
        assert_eq!(probe.value(), "lxc:abc123:demo");
    }

    #[test]
    fn scope_suffix_is_stripped() {
        assert_eq!(extract_name("demo.scope"), Some("demo".to_owned()));
    }

    #[test]
    fn service_suffix_is_stripped() {
        assert_eq!(extract_name("demo.service"), Some("demo".to_owned()));
    }

    #[test]
    fn name_with_dots_is_preserved() {
        // Plain liblxc allows dots in names. Greedy consumption up to
        // the first non-name byte preserves the full dotted name.
        assert_eq!(
            match_lxc_marker("/lxc.payload.foo.bar/init.scope"),
            Some("foo.bar".to_owned())
        );
    }

    #[test]
    fn name_with_hyphen_is_preserved() {
        assert_eq!(
            match_lxc_marker("/lxc.payload.my-container"),
            Some("my-container".to_owned())
        );
    }

    #[test]
    fn name_with_underscore_is_preserved() {
        assert_eq!(
            match_lxc_marker("/lxc.payload.my_ct"),
            Some("my_ct".to_owned())
        );
    }

    #[test]
    fn empty_name_is_rejected() {
        // `/lxc.payload.` with nothing after.
        assert_eq!(match_lxc_marker("/lxc.payload./leftover"), None);
    }

    #[test]
    fn legacy_substring_in_share_path_rejected() {
        // Would false-match under substring semantics; prefix-only match
        // confines the legacy pattern to genuine cgroup paths.
        assert_eq!(match_lxc_marker("/usr/share/lxc/templates/download"), None);
    }

    #[test]
    fn lxc_templates_hyphenated_path_rejected() {
        // `/var/lib/lxc-templates/...` — no `/lxc/` prefix (it's
        // `/lxc-templates/`) and no `.payload.` / `.monitor.` tokens.
        assert_eq!(match_lxc_marker("/var/lib/lxc-templates/download"), None);
    }

    #[test]
    fn payload_substring_in_deeper_path_still_matches() {
        // Mountinfo can carry the payload cgroup as part of a longer
        // path (e.g. bind-mount source inside /sys/fs/cgroup). Substring
        // match is what makes that case work.
        assert_eq!(
            match_lxc_marker("/sys/fs/cgroup/lxc.payload.demo/memory"),
            Some("demo".to_owned())
        );
    }

    #[test]
    fn name_stops_at_slash() {
        // Path-traversal guard: `/` is not a valid name byte, so it
        // cannot sneak into the composed `lxc:<mid>:<name>` value.
        assert_eq!(match_lxc_marker("/lxc.payload.a/b"), Some("a".to_owned()),);
    }

    #[test]
    fn name_stops_at_whitespace() {
        assert_eq!(
            match_lxc_marker("/lxc.payload.bad name"),
            Some("bad".to_owned()),
        );
    }

    #[test]
    fn machine_id_missing_yields_none() {
        let mut cg = NamedTempFile::new().unwrap();
        cg.write_all(b"0::/lxc.payload.demo\n").unwrap();
        let mi = NamedTempFile::new().unwrap();
        // Use a path that won't exist.
        let missing = cg.path().with_extension("definitely-not-there");
        let probe = LxcId::new()
            .with_cgroup(cg.path())
            .with_mountinfo(mi.path())
            .with_machine_id(missing)
            .probe()
            .unwrap();
        assert!(probe.is_none());
    }

    #[test]
    fn machine_id_sentinel_yields_none() {
        let probe = probe_with("0::/lxc.payload.demo\n", "", "uninitialized\n");
        assert!(probe.is_none());
    }

    #[test]
    fn machine_id_empty_yields_none() {
        let probe = probe_with("0::/lxc.payload.demo\n", "", "   \n");
        assert!(probe.is_none());
    }

    #[test]
    fn no_markers_yields_none() {
        let probe = probe_with("0::/user.slice/user-1000.slice\n", "", "abc123\n");
        assert!(probe.is_none());
    }

    #[test]
    fn cgroup_wins_over_mountinfo() {
        // When both files carry a marker, scan_file is consulted for
        // cgroup first. The result should reflect the cgroup's name.
        let probe = probe_with(
            "0::/lxc.payload.from-cgroup\n",
            "1 2 0:0 / /host rw - overlay overlay rw,lowerdir=/lxc.payload.from-mountinfo/rootfs\n",
            "abc123\n",
        )
        .unwrap();
        assert_eq!(probe.value(), "lxc:abc123:from-cgroup");
    }

    #[test]
    fn mountinfo_fallback_when_cgroup_missing() {
        let probe = probe_with(
            "0::/\n",
            "1 2 0:0 / /host rw - overlay overlay rw,lowerdir=/lxc.payload.demo/rootfs\n",
            "abc123\n",
        )
        .unwrap();
        assert_eq!(probe.value(), "lxc:abc123:demo");
    }

    #[test]
    fn composed_value_is_stable_across_calls() {
        let mut cg = NamedTempFile::new().unwrap();
        cg.write_all(b"0::/lxc.payload.demo\n").unwrap();
        let mi = NamedTempFile::new().unwrap();
        let mut id = NamedTempFile::new().unwrap();
        id.write_all(b"abc123\n").unwrap();
        let src = LxcId::new()
            .with_cgroup(cg.path())
            .with_mountinfo(mi.path())
            .with_machine_id(id.path());
        let a = src.probe().unwrap().unwrap();
        let b = src.probe().unwrap().unwrap();
        assert_eq!(a.value(), b.value());
        assert_eq!(a.value(), "lxc:abc123:demo");
    }

    #[test]
    fn mountinfo_past_cap_is_ignored() {
        // Regression guard: the 2 MiB cap protects against corrupt or
        // adversarial mountinfo. Writing a padded prefix followed by a
        // real marker past the cap must not match — otherwise the
        // `file.take(cap)` wrapper was accidentally removed.
        let mut mi = NamedTempFile::new().unwrap();
        // Pad with non-matching lines just over 2 MiB.
        let padding_line = "1 2 0:0 / /pad rw - overlay overlay rw,lowerdir=/var/lib/foo/bar\n";
        let cap = usize::try_from(MAX_MOUNTINFO_BYTES).unwrap();
        let mut written = 0usize;
        while written <= cap {
            mi.write_all(padding_line.as_bytes()).unwrap();
            written += padding_line.len();
        }
        // Now write the only LXC marker, past the cap.
        mi.write_all(
            b"9 9 0:0 / /late rw - overlay overlay rw,lowerdir=/lxc.payload.hidden/rootfs\n",
        )
        .unwrap();
        mi.flush().unwrap();
        let cg = NamedTempFile::new().unwrap();
        let mut id = NamedTempFile::new().unwrap();
        id.write_all(b"abc123\n").unwrap();
        let probe = LxcId::new()
            .with_cgroup(cg.path())
            .with_mountinfo(mi.path())
            .with_machine_id(id.path())
            .probe()
            .unwrap();
        assert!(probe.is_none(), "marker past cap must not match");
    }
}
