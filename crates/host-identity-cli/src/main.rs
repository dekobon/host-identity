//! `hostid` binary entry point. All logic lives in the
//! [`host_identity_cli`] library so tooling (the workspace `xtask`
//! that generates man pages) can reuse the same `clap` definition.

use std::process::ExitCode;

fn main() -> ExitCode {
    host_identity_cli::run()
}
