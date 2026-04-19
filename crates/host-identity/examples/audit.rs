//! Walk the default chain without short-circuiting and pretty-print every
//! source's outcome. Useful for diagnosing why a host resolves to the ID
//! it does — or for confirming that several sources agree.
//!
//! ```bash
//! cargo run --example audit
//! ```

use host_identity::ResolveOutcome;

fn main() {
    let outcomes = host_identity::resolve_all();

    println!("walked {} source(s):", outcomes.len());
    for (i, outcome) in outcomes.iter().enumerate() {
        let kind = outcome.source();
        match outcome {
            ResolveOutcome::Found(id) => {
                println!("  {i:>2}. {kind:<28} -> {}", id.summary());
            }
            ResolveOutcome::Skipped(_) => {
                println!("  {i:>2}. {kind:<28} -> (skipped)");
            }
            ResolveOutcome::Errored(_, err) => {
                println!("  {i:>2}. {kind:<28} -> ERROR {err}");
            }
        }
    }

    // The first `Found` is what `resolve()` would have returned.
    if let Some(first) = outcomes.iter().find_map(ResolveOutcome::host_id) {
        println!("\nwinner: {}", first.summary());
    } else {
        println!("\nno source produced a value on this host");
    }
}
