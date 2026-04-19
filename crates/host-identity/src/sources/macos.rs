//! macOS: `IOPlatformUUID` from `IOPlatformExpertDevice`.
//!
//! Authoritative references:
//!
//! - [`ioreg(8)`](https://keith.github.io/xcode-man-pages/ioreg.8.html) —
//!   the I/O Kit registry browser used to read the platform device tree.
//!   This source shells out to `/usr/sbin/ioreg -rd1 -c IOPlatformExpertDevice`.
//! - [Apple developer documentation: IOKit](https://developer.apple.com/documentation/iokit)
//!   — `IOPlatformExpertDevice` is the device object representing the Mac
//!   hardware platform; `IOPlatformUUID` is the per-system hardware UUID
//!   it publishes as a registry property.
//!
//! # Blocking behaviour
//!
//! This source spawns `ioreg` and waits on it synchronously. On a healthy
//! system the call returns in milliseconds, but a wedged IOKit subsystem
//! can leave the child blocked indefinitely. Callers that need a bounded
//! resolver latency should run [`crate::Resolver::resolve`] on a worker
//! with its own timeout and fall back to the next source on expiry.

use std::process::Command;

use crate::error::Error;
use crate::source::{Probe, Source, SourceKind};
use crate::sources::util::normalize;

/// Reads `IOPlatformUUID` via `ioreg -rd1 -c IOPlatformExpertDevice`.
#[derive(Debug, Default, Clone)]
pub struct IoPlatformUuid {
    _priv: (),
}

impl IoPlatformUuid {
    /// Construct the source.
    #[must_use]
    pub fn new() -> Self {
        Self { _priv: () }
    }
}

impl Source for IoPlatformUuid {
    fn kind(&self) -> SourceKind {
        SourceKind::IoPlatformUuid
    }

    fn probe(&self) -> Result<Option<Probe>, Error> {
        let output = Command::new("/usr/sbin/ioreg")
            .args(["-rd1", "-c", "IOPlatformExpertDevice"])
            .output()
            .map_err(|e| Error::Platform {
                source_kind: SourceKind::IoPlatformUuid,
                reason: format!("ioreg: {e}"),
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::debug!(
                "host-identity: ioreg exited with {}: {}",
                output.status,
                stderr.trim()
            );
            return Ok(None);
        }
        let Ok(stdout) = std::str::from_utf8(&output.stdout) else {
            return Ok(None);
        };
        Ok(extract_io_platform_uuid(stdout)
            .as_deref()
            .and_then(normalize)
            .map(|v| Probe::new(SourceKind::IoPlatformUuid, v)))
    }
}

fn extract_io_platform_uuid(ioreg_output: &str) -> Option<String> {
    for line in ioreg_output.lines() {
        let trimmed = line
            .trim()
            .trim_start_matches(|c: char| c == '|' || c.is_whitespace());
        if let Some(rest) = trimmed.strip_prefix("\"IOPlatformUUID\"") {
            if let Some(eq) = rest.find('=') {
                let value = rest[eq + 1..].trim().trim_matches('"');
                if !value.is_empty() {
                    return Some(value.to_owned());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ioreg_line() {
        let sample = r#"
            | {
            |   "IOPlatformSerialNumber" = "C02XXXX"
            |   "IOPlatformUUID" = "12345678-1234-1234-1234-123456789ABC"
            | }
        "#;
        assert_eq!(
            extract_io_platform_uuid(sample).as_deref(),
            Some("12345678-1234-1234-1234-123456789ABC")
        );
    }

    #[test]
    fn returns_none_when_no_uuid_line_present() {
        let sample = r#"
            | "IOPlatformSerialNumber" = "C02XXXX"
            | "IOBusyInterest" = "IOCommand is not serializable"
        "#;
        assert_eq!(extract_io_platform_uuid(sample), None);
    }

    #[test]
    fn rejects_empty_quoted_value() {
        let sample = r#"| "IOPlatformUUID" = """#;
        assert_eq!(extract_io_platform_uuid(sample), None);
    }

    #[test]
    fn ignores_keys_that_merely_start_with_io_platform_uuid() {
        // The prefix literal includes the closing quote, so adjacent keys
        // like `IOPlatformUUIDHash` must not be matched. A refactor that
        // drops the closing quote from the strip_prefix would silently
        // accept these.
        let sample = r#"| "IOPlatformUUIDHash" = "deadbeef""#;
        assert_eq!(extract_io_platform_uuid(sample), None);
    }
}
