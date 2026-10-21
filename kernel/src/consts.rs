// Movement constants shared by the kernel and the PTX image. Both crates
// `include!` this file at their root, so the device cannot drift from the
// values the Lean proofs name (`ai_town_kernel.MILLI`, `COLLISION_SQ`, ...).

/// One tile, in milli-tiles.
pub const MILLI: u32 = 1000;
/// 0.75 tiles per second.
pub const SPEED: u32 = 750;
pub const COLLISION_SQ: u64 = 750 * 750;
pub const FACE_BIAS: u32 = 1000;
