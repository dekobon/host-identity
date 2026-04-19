# Changelog

All notable changes to this project are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `LxcId` source for LXC/LXD containers. Reads the container name from
  `/proc/self/cgroup` (falling back to `/proc/self/mountinfo`) and
  salts it with `/etc/machine-id` so two hosts running identically
  named containers do not collide. Exposed as `SourceKind::Lxc` with
  the `"lxc"` identifier and inserted immediately after `ContainerId`
  in `default_chain` and `network_default_chain`. Gated behind the
  existing `container` feature on Linux. Fixes #1.
