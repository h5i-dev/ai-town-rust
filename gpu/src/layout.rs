//! Fixed widths shared by the host arena and the device kernel.
//!
//! The device crate includes this file with `#[path]`, so the host packer and
//! the kernel's capacity check cannot drift. Keep the file free of `std`.

/// Upper bound on agents in one seek batch.
pub const MAX_AGENTS: usize = 1_000_000;
/// Words in one seek row: `x`, `y`, target tile `x`, target tile `y`.
pub const ROW_WORDS: usize = 4;
