//! Workspace task runner. Today it only generates man pages for the
//! `hostid` CLI; extend with more `cargo xtask <thing>` subcommands as
//! the project needs them.
#![allow(missing_docs)]
#![allow(clippy::pedantic)]

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use clap::CommandFactory;

fn main() -> io::Result<()> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask must be a workspace member")
        .to_path_buf();

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
        .source(format!("hostid {}", host_identity_cli::VERSION))
        .manual("hostid Manual".to_string());

    let mut buffer = Vec::<u8>::new();
    man.render(&mut buffer)?;
    fs::write(out_dir.join(format!("{name}.1")), buffer)?;
    Ok(())
}
