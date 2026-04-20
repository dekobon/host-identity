//! Workspace task runner.
//!
//! - `cargo xtask` (no args) generates man pages for the `host-identity` CLI.
//! - `cargo xtask shellspec [--] [shellspec args...]` builds the CLI and
//!   runs the `spec/` suite against the freshly-built debug binary.
#![allow(missing_docs)]
#![allow(clippy::pedantic)]

use std::{
    env,
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use clap::CommandFactory;

fn main() -> ExitCode {
    let workspace_root = workspace_root();
    let mut args = env::args_os().skip(1);
    // `to_str()` returns None for non-UTF-8 — route those to the unknown
    // arm so a stray non-UTF-8 byte cannot silently invoke man-page
    // generation.
    match args.next().as_deref().map(OsStr::to_str) {
        None => run_manpages(&workspace_root).map_or_else(io_exit, |()| ExitCode::SUCCESS),
        Some(Some("shellspec")) => run_shellspec(&workspace_root, &args.collect::<Vec<_>>()),
        Some(other) => {
            let label = other.unwrap_or("<non-utf8>");
            eprintln!("xtask: unknown subcommand `{label}` (expected none or `shellspec`)");
            ExitCode::from(2)
        }
    }
}

fn io_exit(e: io::Error) -> ExitCode {
    eprintln!("xtask: {e}");
    ExitCode::FAILURE
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask must be a workspace member")
        .to_path_buf()
}

fn run_manpages(workspace_root: &Path) -> io::Result<()> {
    let out_dir = workspace_root.join("man");
    fs::create_dir_all(&out_dir)?;

    let cmd = host_identity_cli::Cli::command();
    render_man_page(&cmd, &out_dir)?;
    render_subcommands(&cmd, cmd.get_name(), &out_dir)?;

    println!("Wrote man pages to {}", out_dir.display());
    Ok(())
}

fn render_subcommands(parent: &clap::Command, prefix: &str, out_dir: &Path) -> io::Result<()> {
    for sub in parent.get_subcommands() {
        if sub.get_name() == "help" {
            continue;
        }
        let full_name: &'static str =
            Box::leak(format!("{}-{}", prefix, sub.get_name()).into_boxed_str());
        let sub_cmd = sub.clone().name(full_name);
        render_man_page(&sub_cmd, out_dir)?;
        render_subcommands(sub, full_name, out_dir)?;
    }
    Ok(())
}

fn render_man_page(cmd: &clap::Command, out_dir: &Path) -> io::Result<()> {
    let name = cmd.get_name().to_string();
    let man = clap_mangen::Man::new(cmd.clone())
        .title(name.to_uppercase())
        .section("1")
        .source(format!("host-identity {}", host_identity_cli::VERSION))
        .manual("host-identity Manual".to_string());

    let mut buffer = Vec::<u8>::new();
    man.render(&mut buffer)?;
    fs::write(out_dir.join(format!("{name}.1")), buffer)?;
    Ok(())
}

fn run_shellspec(workspace_root: &Path, extra: &[std::ffi::OsString]) -> ExitCode {
    // `--locked` matches the CI step so a developer's local lockfile drift
    // surfaces here rather than being papered over by an implicit update.
    let build = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(["build", "-p", "host-identity-cli", "--locked"])
        .current_dir(workspace_root)
        .status();
    match build {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("xtask: `cargo build -p host-identity-cli` failed ({s})");
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("xtask: could not invoke cargo: {e}");
            return ExitCode::FAILURE;
        }
    }

    // Honour CARGO_TARGET_DIR so the spec suite finds the same binary
    // Cargo just produced.
    let target_dir = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root.join("target"));
    let bin = target_dir.join("debug").join(binary_name());
    if !bin.is_file() {
        eprintln!("xtask: expected CLI binary at {}", bin.display());
        return ExitCode::FAILURE;
    }

    let status = Command::new("shellspec")
        .args(extra)
        .current_dir(workspace_root)
        .env("HOST_IDENTITY_BIN", &bin)
        .status();
    match status {
        Ok(s) => s
            .code()
            .and_then(|c| u8::try_from(c).ok())
            .map_or(ExitCode::FAILURE, ExitCode::from),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            eprintln!(
                "xtask: `shellspec` not found on PATH. Install it from \
                 https://github.com/shellspec/shellspec and re-run."
            );
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("xtask: failed to spawn shellspec: {e}");
            ExitCode::FAILURE
        }
    }
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "host-identity.exe"
    } else {
        "host-identity"
    }
}
