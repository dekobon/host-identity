# Changelog

All notable changes to this project are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [1.0.0] - 2026-04-19

Initial stable release. The public API surface of the library
(`resolve`, `resolve_with`, `Source`, `SourceKind`, `default_chain`,
`network_default_chain`, the `HttpTransport` trait) is now covered
by semver.

### Added

- Core resolution pipeline: `resolve()` walks an ordered chain of
  identity sources, short-circuits on the first usable value, rejects
  known sentinels (empty files, systemd's `uninitialized`, all-zero
  UUIDs), and wraps the winning raw identifier as a UUID v5 under a
  stable project namespace. Callers that need a different policy can
  compose their own chain via `resolve_with`.
- Platform identity sources: `/etc/machine-id` and
  `/var/lib/dbus/machine-id` on Linux, SMBIOS `product_uuid` on Linux,
  `IOPlatformUUID` on macOS, `HKLM\...\Cryptography\MachineGuid` on
  Windows, `/etc/hostid` and `kenv smbios.system.uuid` on FreeBSD,
  `sysctl kern.hostid` on NetBSD/OpenBSD, `hostid(1)` on
  illumos/Solaris.
- Container-scope sources (behind the `container` feature): runtime
  ID from `/proc/self/mountinfo` and LXC/LXD container name from
  `/proc/self/cgroup`, both salted with `/etc/machine-id` so identical
  container names on different hosts do not collide.
- Cloud metadata sources (behind `cloud-aws`, `cloud-gcp`,
  `cloud-azure`, `cloud-digitalocean`, `cloud-hetzner`, `cloud-oci`):
  AWS IMDSv2, GCP Compute Engine, Azure IMDS, DigitalOcean, Hetzner,
  OCI. Network sources are generic over a caller-supplied
  `HttpTransport` — the crate ships no HTTP client.
- Kubernetes sources (behind the `kubernetes` feature): pod UID from
  `/proc/self/mountinfo`, service-account namespace, and downward-API
  projected files.
- Operator overrides: `HOST_IDENTITY` environment variable and a
  file-path override, both consulted ahead of every platform source.
- `host-identity-cli` (`hostid`) binary: `resolve` (default),
  `audit` (walk every source without short-circuiting and report each
  outcome), `sources` (list compiled-in sources), and `--version`.
  Man pages are generated from the clap schema.
- Packaging for Debian/Ubuntu (`.deb`), RHEL/Fedora/Amazon Linux
  (`.rpm`), Alpine (`.apk`), FreeBSD (`.pkg`), Homebrew, and Scoop.
  Release artefacts ship with CycloneDX SBOMs, minisign signatures
  over `SHA256SUMS`, and SLSA build provenance.
