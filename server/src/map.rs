//! The gentle town, shared by the collision mask and the canvas.

use ai_town_kernel::MAX_DIM;
use serde::Deserialize;

const GENTLE: &str = include_str!("../../assets/gentle.json");

#[derive(Clone, Debug)]
pub struct MapFile {
    pub width: u32,
    pub height: u32,
    pub mask: Vec<u8>,
    pub json: &'static str,
}

#[derive(Deserialize)]
struct RawMap {
    width: u32,
    height: u32,
    #[serde(rename = "objectTiles")]
    object_tiles: Vec<Vec<Vec<i64>>>,
}

pub fn gentle() -> Result<MapFile, String> {
    let raw: RawMap = serde_json::from_str(GENTLE).map_err(|e| e.to_string())?;
    if raw.width == 0 || raw.height == 0 || raw.width > MAX_DIM || raw.height > MAX_DIM {
        return Err("map dimensions are out of range".into());
    }
    let mask = mask_from_objects(&raw.object_tiles, raw.width, raw.height);
    let cells = (raw.width as usize) * (raw.height as usize);
    if mask.len() != cells.div_ceil(8) {
        return Err("mask length does not match the map".into());
    }
    Ok(MapFile { width: raw.width, height: raw.height, mask, json: GENTLE })
}

/// Bit `tx * height + ty` is set when any object layer has a tile other than -1.
pub fn mask_from_objects(layers: &[Vec<Vec<i64>>], width: u32, height: u32) -> Vec<u8> {
    let cells = (width as usize) * (height as usize);
    let mut mask = vec![0u8; cells.div_ceil(8)];
    for layer in layers {
        let mut x = 0u32;
        while x < width && (x as usize) < layer.len() {
            let col = &layer[x as usize];
            let mut y = 0u32;
            while y < height && (y as usize) < col.len() {
                if col[y as usize] != -1 {
                    let i = (x as usize) * (height as usize) + (y as usize);
                    mask[i / 8] |= 1 << (i % 8);
                }
                y += 1;
            }
            x += 1;
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_town_kernel::mask_blocked;

    #[test]
    fn gentle_mask_matches_the_object_layer() {
        let map = gentle().expect("gentle.json");
        assert_eq!((map.width, map.height), (64, 48));
        assert!(mask_blocked(&map.mask, map.width, map.height, 0, 0));
        let mut free = false;
        let mut blocked = false;
        let mut x = 0u32;
        while x < map.width {
            let mut y = 0u32;
            while y < map.height {
                if mask_blocked(&map.mask, map.width, map.height, x * 1000, y * 1000) {
                    blocked = true;
                } else {
                    free = true;
                }
                y += 1;
            }
            x += 1;
        }
        assert!(free && blocked);
    }

    #[test]
    fn a_single_object_sets_one_bit() {
        let layers = vec![vec![vec![-1, 7], vec![-1, -1]]];
        let mask = mask_from_objects(&layers, 2, 2);
        assert!(mask_blocked(&mask, 2, 2, 0, 1000));
        assert!(!mask_blocked(&mask, 2, 2, 0, 0));
        assert!(!mask_blocked(&mask, 2, 2, 1000, 0));
    }
}
