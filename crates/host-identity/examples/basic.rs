//! Minimal example: resolve a host ID using the platform default chain.
//!
//! ```bash
//! cargo run --example basic
//! ```

fn main() -> Result<(), host_identity::Error> {
    let id = host_identity::resolve()?;

    println!("uuid:    {id}");
    println!("summary: {}", id.summary());
    println!("source:  {}", id.source());
    println!("in_container: {}", id.in_container());

    Ok(())
}
