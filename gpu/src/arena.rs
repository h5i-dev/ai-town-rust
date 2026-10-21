//! Host SoA for one seek batch. Device code never calls these functions.
//!
//! Each agent is four `u32`s (`x`, `y`, target tile x, target tile y), stored
//! as columns so the kernel can take the same slices as [`ai_town_kernel::seek_one`].

use crate::layout::{MAX_AGENTS, ROW_WORDS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeekRow {
    pub x: u32,
    pub y: u32,
    pub tx: u32,
    pub ty: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LayoutError {
    Capacity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeekArena {
    pub xs: Vec<u32>,
    pub ys: Vec<u32>,
    pub txs: Vec<u32>,
    pub tys: Vec<u32>,
}

impl SeekArena {
    /// Pack `rows` in input order. `capacity` is the caller's batch limit and
    /// cannot exceed [`MAX_AGENTS`].
    pub fn pack(rows: &[SeekRow], capacity: usize) -> Result<Self, LayoutError> {
        if rows.len() > capacity || capacity > MAX_AGENTS {
            return Err(LayoutError::Capacity);
        }
        let n = rows.len();
        let mut arena = Self {
            xs: Vec::with_capacity(n),
            ys: Vec::with_capacity(n),
            txs: Vec::with_capacity(n),
            tys: Vec::with_capacity(n),
        };
        let mut i = 0;
        while i < n {
            arena.xs.push(rows[i].x);
            arena.ys.push(rows[i].y);
            arena.txs.push(rows[i].tx);
            arena.tys.push(rows[i].ty);
            i += 1;
        }
        let _row_words: usize = ROW_WORDS;
        debug_assert_eq!(_row_words, 4);
        Ok(arena)
    }

    pub fn unpack(&self) -> Vec<SeekRow> {
        let n = self.xs.len().min(self.ys.len()).min(self.txs.len()).min(self.tys.len());
        let mut rows = Vec::with_capacity(n);
        let mut i = 0;
        while i < n {
            rows.push(SeekRow { x: self.xs[i], y: self.ys[i], tx: self.txs[i], ty: self.tys[i] });
            i += 1;
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_roundtrip_and_input_order() {
        let rows = vec![
            SeekRow { x: 10, y: 20, tx: 3, ty: 4 },
            SeekRow { x: 0, y: 1000, tx: 1, ty: 0 },
        ];
        let packed = SeekArena::pack(&rows, 2).unwrap();
        assert_eq!(packed.unpack(), rows);
        assert_eq!(packed.xs, vec![10, 0]);
        assert_eq!(SeekArena::pack(&rows, 1).err(), Some(LayoutError::Capacity));
        assert_eq!(SeekArena::pack(&[], MAX_AGENTS + 1).err(), Some(LayoutError::Capacity));
        assert_eq!(SeekArena::pack(&[], 0).unwrap().unpack(), Vec::new());
        assert!(SeekArena::pack(&[], MAX_AGENTS).is_ok());
    }

    #[test]
    fn separated_agents_match_a_joint_scan() {
        let rows = vec![
            SeekRow { x: 0, y: 0, tx: 5, ty: 0 },
            SeekRow { x: 4000, y: 0, tx: 0, ty: 0 },
        ];
        let packed = SeekArena::pack(&rows, 2).unwrap();
        let together = ai_town_kernel::scalar_seek(&packed.xs, &packed.ys, &packed.txs, &packed.tys, 200, &[], 16, 16);
        let mut split_x = Vec::new();
        let mut split_y = Vec::new();
        let mut i = 0;
        while i < rows.len() {
            let one = SeekArena::pack(&rows[i..i + 1], 1).unwrap();
            let (x, y) = ai_town_kernel::scalar_seek(&one.xs, &one.ys, &one.txs, &one.tys, 200, &[], 16, 16);
            split_x.extend(x);
            split_y.extend(y);
            i += 1;
        }
        assert_eq!(together, (split_x, split_y));
    }
}
