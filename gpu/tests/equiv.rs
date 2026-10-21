use ai_town_gpu::Device;
use ai_town_kernel::{scalar_seek, set_blocked};
use std::sync::Mutex;

fn with_device<T>(body: impl FnOnce(&Device) -> T) -> T {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let dev = Device::open().unwrap_or_else(|err| panic!("cuda {}: {}", err.op, err.code));
    body(&dev)
}

fn assert_match(xs: &[u32], ys: &[u32], txs: &[u32], tys: &[u32], step: u32, mask: &[u8], width: u32, height: u32) {
    let cpu = scalar_seek(xs, ys, txs, tys, step, mask, width, height);
    let gpu = with_device(|dev| {
        dev.seek(xs, ys, txs, tys, step, mask, width, height)
            .unwrap_or_else(|err| panic!("seek {}: {}", err.op, err.code))
    });
    assert_eq!(cpu, gpu);
}

#[test]
fn empty_seek_is_empty() {
    let gpu = with_device(|dev| dev.seek(&[], &[], &[], &[], 200, &[], 4, 4).unwrap_or_else(|err| panic!("{} {}", err.op, err.code)));
    assert_eq!(gpu, (Vec::new(), Vec::new()));
}

#[test]
fn matches_scalar_on_open_ground_blocked_tiles_and_crowding() {
    let dev_mask = {
        let cells = 16 * 16;
        let mut mask = vec![0u8; (cells - 1) / 8 + 1];
        set_blocked(&mut mask, 16, 16, 1, 0);
        mask
    };
    assert_match(&[0, 1000, 2000, 5000], &[0, 0, 1000, 4000], &[5, 5, 0, 1], &[0, 2, 4, 1], 400, &dev_mask, 16, 16);
    assert_match(&[0], &[0], &[3], &[0], 5000, &dev_mask, 16, 16);
    // 700 milli apart is inside the 750 radius, so the step toward the neighbor is refused.
    assert_match(&[0, 700], &[0, 0], &[5, 0], &[0, 0], 200, &dev_mask, 16, 16);
    // Far enough that the step is free.
    assert_match(&[0, 3000], &[0, 0], &[5, 0], &[0, 0], 200, &dev_mask, 16, 16);
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut txs = Vec::new();
    let mut tys = Vec::new();
    let mut i = 0u32;
    while i < 200 {
        xs.push((i % 16) * 1000 + (i % 7) * 15);
        ys.push((i / 16) * 1000 + (i % 5) * 20);
        txs.push((i * 3) % 16);
        tys.push((i * 5) % 16);
        i += 1;
    }
    assert_match(&xs, &ys, &txs, &tys, 180, &dev_mask, 16, 16);
}
