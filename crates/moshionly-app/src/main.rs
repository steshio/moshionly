//! Moshionly desktop app entry point.
//!
//! The editor is assembled crate by crate (see `docs/architecture.md`). Until the
//! window lands this binary only reports its version, so the workspace always builds.

fn main() {
    println!("Moshionly {}", env!("CARGO_PKG_VERSION"));
    println!("Open-source motion design for the desktop. The editor is under construction.");
}
