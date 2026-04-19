//! Build a resolver from a config-supplied list of source identifiers.
//!
//! The config format is intentionally plain here — just one identifier
//! per line — so the example doesn't take a serde dependency. A real app
//! would parse TOML/YAML/JSON and pass the resulting `Vec<String>` to
//! [`host_identity::resolver_from_ids`].
//!
//! ```bash
//! echo 'env-override
//! machine-id
//! dmi' | cargo run --example config_driven
//! ```

use std::io::{self, BufRead, Write};

use host_identity::{ResolveOutcome, UnknownSourceError, resolver_from_ids};

fn main() -> io::Result<()> {
    let ids = read_ids();
    if ids.is_empty() {
        writeln!(
            io::stderr(),
            "no source identifiers on stdin; expected one per line"
        )?;
        std::process::exit(2);
    }

    let resolver = resolver_from_ids(&ids).unwrap_or_else(|err| {
        let _ = writeln!(io::stderr(), "{}", describe_build_error(&err));
        std::process::exit(1);
    });

    println!("chain: {:?}", resolver.source_kinds());
    match resolver.resolve() {
        Ok(id) => println!("resolved: {}", id.summary()),
        Err(err) => {
            // resolve_all gives a nicer diagnostic — print it so the
            // operator can see why nothing matched.
            for outcome in resolver.resolve_all() {
                if let ResolveOutcome::Errored(kind, e) = outcome {
                    writeln!(io::stderr(), "  {kind}: {e}")?;
                }
            }
            writeln!(io::stderr(), "resolve failed: {err}")?;
            std::process::exit(1);
        }
    }

    Ok(())
}

// `map_while(Result::ok)` stops at the first I/O error silently —
// acceptable for an example consuming stdin, but a real app that
// cannot tolerate truncated configs should `collect::<io::Result<_>>()`.
fn read_ids() -> Vec<String> {
    io::stdin()
        .lock()
        .lines()
        .map_while(Result::ok)
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

fn describe_build_error(err: &UnknownSourceError) -> String {
    match err {
        UnknownSourceError::Unknown(id) => format!("unknown source identifier: `{id}`"),
        UnknownSourceError::RequiresPath(id) => format!(
            "source `{id}` requires a caller-supplied path; \
             push it manually with its typed constructor"
        ),
        UnknownSourceError::RequiresTransport(id) => format!(
            "source `{id}` is a cloud source; use \
             resolver_from_ids_with_transport with an HTTP client"
        ),
        UnknownSourceError::FeatureDisabled(id, feat) => format!(
            "source `{id}` requires the `{feat}` feature, \
             which isn't enabled in this build"
        ),
    }
}
