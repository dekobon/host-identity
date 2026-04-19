//! Helpers shared by source implementations.

use std::io::{self, Read};
use std::path::Path;

/// Upper bound on identity-file reads (64 KiB). Every source the crate
/// ships reads a file that should contain one short identifier — a
/// UUID, a numeric hostid, a 64-hex container ID, or a short cgroup
/// line. Capping the read keeps a corrupted or adversarial file from
/// exhausting memory before `trim` / `classify` rejects it.
pub(crate) const MAX_ID_FILE_BYTES: u64 = 64 * 1024;

/// Read a file, capped at [`MAX_ID_FILE_BYTES`]. Files larger than the
/// cap are truncated silently — the raw content is still passed through
/// [`classify`], which rejects anything that isn't a clean identifier
/// anyway.
pub(crate) fn read_capped(path: &Path) -> io::Result<String> {
    let file = std::fs::File::open(path)?;
    let mut buf = String::new();
    file.take(MAX_ID_FILE_BYTES).read_to_string(&mut buf)?;
    Ok(buf)
}

/// Normalise a base URL by trimming trailing `/` characters, so that
/// concatenation with a leading-slash path never produces `//`.
///
/// Shared by cloud-source constructors that accept a caller-supplied
/// base URL.
#[cfg(feature = "_transport")]
pub(crate) fn trim_trailing_slashes(base_url: impl Into<String>) -> String {
    let mut base_url = base_url.into();
    let trimmed_len = base_url.trim_end_matches('/').len();
    base_url.truncate(trimmed_len);
    base_url
}

/// systemd writes this literal string to `/etc/machine-id` during early
/// boot and overmounts it with the real ID once provisioning completes.
/// Sources that read machine-id-shaped files must reject it; otherwise
/// every host in that window hashes to the same UUID.
pub const UNINITIALIZED_SENTINEL: &str = "uninitialized";

/// Trim the raw value and reject empty / sentinel strings.
///
/// Returns `Some(trimmed)` when the value is usable, `None` otherwise.
/// Source authors should call this on every file or command output before
/// wrapping it in a [`crate::Probe`].
#[must_use]
pub fn normalize(raw: &str) -> Option<&str> {
    match classify(raw) {
        NormalizeOutcome::Usable(v) => Some(v),
        _ => None,
    }
}

/// Outcome of [`classify`] — the full-detail variant of [`normalize`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NormalizeOutcome<'a> {
    /// A usable, trimmed value.
    Usable(&'a str),
    /// Empty or whitespace-only.
    Empty,
    /// The `uninitialized` sentinel.
    Sentinel,
}

/// Like [`normalize`] but distinguishes empty input from the sentinel value.
pub(crate) fn classify(raw: &str) -> NormalizeOutcome<'_> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return NormalizeOutcome::Empty;
    }
    if trimmed.eq_ignore_ascii_case(UNINITIALIZED_SENTINEL) {
        return NormalizeOutcome::Sentinel;
    }
    NormalizeOutcome::Usable(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_rejects_sentinels() {
        assert_eq!(normalize(""), None);
        assert_eq!(normalize("   "), None);
        assert_eq!(normalize("uninitialized"), None);
        assert_eq!(normalize("UNINITIALIZED\n"), None);
        assert_eq!(normalize("  abc123\n"), Some("abc123"));
    }

    #[test]
    fn classify_distinguishes_empty_from_sentinel() {
        assert_eq!(classify(""), NormalizeOutcome::Empty);
        assert_eq!(classify("   \n\t"), NormalizeOutcome::Empty);
        assert_eq!(classify("uninitialized"), NormalizeOutcome::Sentinel);
        assert_eq!(classify("UNINITIALIZED\n"), NormalizeOutcome::Sentinel);
        assert_eq!(classify("  abc123\n"), NormalizeOutcome::Usable("abc123"));
    }
}
