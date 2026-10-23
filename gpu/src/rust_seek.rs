//! Launch the PTX image of the shared movement kernels, and the `seek_one` baseline.
//!
//! The handwritten cubin in `seek.cu` stays as a baseline. This launcher uses
//! the same driver API and does not depend on the `cust` crate. The CUDA
//! context is tied to the thread that opened it. `step` runs the sequential
//! tick on one thread. `rounds` runs the two-phase rule, one thread per agent,
//! and leaves positions on the device across rounds.

use crate::{GpuError, MAX_AGENTS};
use std::ffi::c_void;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::path::Path;
use std::time::Instant;

type CUresult = i32;
type CUdevice = i32;
type CUdeviceptr = u64;
type CUcontext = *mut c_void;
type CUmodule = *mut c_void;
type CUfunction = *mut c_void;
type CUevent = *mut c_void;

#[link(name = "cuda")]
unsafe extern "C" {
    fn cuInit(flags: u32) -> CUresult;
    fn cuDeviceGet(device: *mut CUdevice, ordinal: i32) -> CUresult;
    fn cuCtxCreate_v2(ctx: *mut CUcontext, flags: u32, dev: CUdevice) -> CUresult;
    fn cuCtxDestroy_v2(ctx: CUcontext) -> CUresult;
    fn cuCtxSetCurrent(ctx: CUcontext) -> CUresult;
    fn cuModuleLoadData(module: *mut CUmodule, image: *const c_void) -> CUresult;
    fn cuModuleUnload(module: CUmodule) -> CUresult;
    fn cuModuleGetFunction(hfunc: *mut CUfunction, module: CUmodule, name: *const i8) -> CUresult;
    fn cuMemAlloc_v2(dptr: *mut CUdeviceptr, bytes: usize) -> CUresult;
    fn cuMemFree_v2(dptr: CUdeviceptr) -> CUresult;
    fn cuMemcpyHtoD_v2(dst: CUdeviceptr, src: *const c_void, bytes: usize) -> CUresult;
    fn cuMemcpyDtoH_v2(dst: *mut c_void, src: CUdeviceptr, bytes: usize) -> CUresult;
    fn cuMemcpyDtoD_v2(dst: CUdeviceptr, src: CUdeviceptr, bytes: usize) -> CUresult;
    fn cuLaunchKernel(
        f: CUfunction,
        grid_x: u32,
        grid_y: u32,
        grid_z: u32,
        block_x: u32,
        block_y: u32,
        block_z: u32,
        shared: u32,
        stream: *mut c_void,
        args: *mut *mut c_void,
        extra: *mut *mut c_void,
    ) -> CUresult;
    fn cuEventCreate(event: *mut CUevent, flags: u32) -> CUresult;
    fn cuEventRecord(event: CUevent, stream: *mut c_void) -> CUresult;
    fn cuEventSynchronize(event: CUevent) -> CUresult;
    fn cuEventElapsedTime(ms: *mut f32, start: CUevent, end: CUevent) -> CUresult;
    fn cuEventDestroy_v2(event: CUevent) -> CUresult;
    fn cuMemsetD32_v2(dst: CUdeviceptr, value: u32, n: usize) -> CUresult;
}

fn check(op: &'static str, code: CUresult) -> Result<(), GpuError> {
    if code == 0 { Ok(()) } else { Err(GpuError { op, code }) }
}

/// Positions and per-row errors from one batch. Error 0 is a successful decision,
/// including a step that was refused and stayed put. Error 1 is a missing row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeekOut {
    pub xs: Vec<u32>,
    pub ys: Vec<u32>,
    pub errors: Vec<u32>,
}

/// Kernel time is the sum of CUDA-event intervals around each launch.
/// Wall time includes the copies.
#[derive(Clone, Debug, PartialEq)]
pub struct SeekReport {
    pub out: SeekOut,
    pub kernel_ms: f32,
    pub wall_ms: f64,
}

/// Positions after one sequential tick. `error` 0 means `tick_moves` ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepOut {
    pub xs: Vec<u32>,
    pub ys: Vec<u32>,
    pub error: u32,
}

/// Kernel time is the CUDA-event interval around the device work.
/// Wall time includes the copies. `rounds` copies once for the whole batch.
#[derive(Clone, Debug, PartialEq)]
pub struct StepReport {
    pub out: StepOut,
    pub kernel_ms: f32,
    pub wall_ms: f64,
}

pub struct RustDevice {
    ctx: CUcontext,
    module: CUmodule,
    func: CUfunction,
    step_func: CUfunction,
    round_func: CUfunction,
    verify_grid: bool,
    workspace: RefCell<GpuBufs>,
    _thread: PhantomData<*const ()>,
}

// A thread-bound device owns one reusable allocation per argument slot.
// Each entry point rewinds the slots; growing a slot is transactional so a
// failed allocation leaves the previous workspace usable on the next call.
struct GpuBufs {
    slots: Vec<(CUdeviceptr, usize)>,
    cursor: usize,
}

impl GpuBufs {
    fn new() -> Self { Self { slots: Vec::new(), cursor: 0 } }

    fn rewind(&mut self) { self.cursor = 0; }

    fn alloc(&mut self, bytes: usize) -> Result<CUdeviceptr, GpuError> {
        let bytes = bytes.max(1);
        let index = self.cursor;
        if let Some(&(ptr, capacity)) = self.slots.get(index) {
            if capacity >= bytes { self.cursor += 1; return Ok(ptr); }
        }
        let mut ptr = 0;
        check("cuMemAlloc", unsafe { cuMemAlloc_v2(&mut ptr, bytes) })?;
        if index < self.slots.len() {
            let old = self.slots[index].0;
            self.slots[index] = (ptr, bytes);
            unsafe { let _ = cuMemFree_v2(old); }
        } else {
            self.slots.push((ptr, bytes));
        }
        self.cursor += 1;
        Ok(ptr)
    }

    fn clear(&mut self) {
        for (ptr, _) in self.slots.drain(..) {
            unsafe { let _ = cuMemFree_v2(ptr); }
        }
        self.cursor = 0;
    }
}

impl Drop for GpuBufs {
    fn drop(&mut self) { self.clear(); }
}

fn h2d<T>(dst: CUdeviceptr, src: &[T]) -> Result<(), GpuError> {
    unsafe {
        if src.is_empty() {
            let zero = 0u8;
            return check("cuMemcpyHtoD", cuMemcpyHtoD_v2(dst, &zero as *const u8 as *const c_void, 1));
        }
        check("cuMemcpyHtoD", cuMemcpyHtoD_v2(dst, src.as_ptr().cast(), std::mem::size_of_val(src)))
    }
}

impl RustDevice {
    /// Load a null-terminated PTX image. `ptx` may already end in 0; an extra 0 is harmless.
    pub fn open_image(ptx: &[u8]) -> Result<Self, GpuError> {
        let mut image = Vec::with_capacity(ptx.len() + 1);
        image.extend_from_slice(ptx);
        image.push(0);
        unsafe {
            check("cuInit", cuInit(0))?;
            let mut dev: CUdevice = 0;
            check("cuDeviceGet", cuDeviceGet(&mut dev, 0))?;
            let mut ctx: CUcontext = std::ptr::null_mut();
            check("cuCtxCreate", cuCtxCreate_v2(&mut ctx, 0, dev))?;
            let mut module: CUmodule = std::ptr::null_mut();
            let loaded = cuModuleLoadData(&mut module, image.as_ptr().cast());
            if loaded != 0 {
                let _ = cuCtxDestroy_v2(ctx);
                return Err(GpuError { op: "cuModuleLoadData", code: loaded });
            }
            let mut func: CUfunction = std::ptr::null_mut();
            let name = c"seek_batch";
            let got = cuModuleGetFunction(&mut func, module, name.as_ptr());
            if got != 0 {
                let _ = cuModuleUnload(module);
                let _ = cuCtxDestroy_v2(ctx);
                return Err(GpuError { op: "cuModuleGetFunction", code: got });
            }
            let mut step_func: CUfunction = std::ptr::null_mut();
            let step_name = c"step_batch";
            let got_step = cuModuleGetFunction(&mut step_func, module, step_name.as_ptr());
            if got_step != 0 {
                let _ = cuModuleUnload(module);
                let _ = cuCtxDestroy_v2(ctx);
                return Err(GpuError { op: "cuModuleGetFunction", code: got_step });
            }
            let mut round_func: CUfunction = std::ptr::null_mut();
            let round_name = c"round_phase";
            let got_round = cuModuleGetFunction(&mut round_func, module, round_name.as_ptr());
            if got_round != 0 {
                let _ = cuModuleUnload(module);
                let _ = cuCtxDestroy_v2(ctx);
                return Err(GpuError { op: "cuModuleGetFunction", code: got_round });
            }
            Ok(RustDevice { ctx, module, func, step_func, round_func, verify_grid: false, workspace: RefCell::new(GpuBufs::new()), _thread: PhantomData })
        }
    }

    /// When on, [`Self::rounds`] copies every grid the device builds back to
    /// the host and refuses the batch unless `ai_town_kernel::grid_wellformed`
    /// accepts it. The copies stall the stream, so leave it off when timing.
    pub fn set_verify_grid(&mut self, on: bool) {
        self.verify_grid = on;
    }

    pub fn open_path(path: &Path) -> Result<Self, GpuError> {
        let bytes = std::fs::read(path).map_err(|_| GpuError { op: "ptx", code: -1 })?;
        Self::open_image(&bytes)
    }

    /// One step. `n` is `min(xs.len(), ys.len())`.
    pub fn seek(
        &self,
        xs: &[u32],
        ys: &[u32],
        txs: &[u32],
        tys: &[u32],
        step: u32,
        mask: &[u8],
        width: u32,
        height: u32,
    ) -> Result<SeekOut, GpuError> {
        Ok(self.seek_repeat(xs, ys, txs, tys, step, mask, width, height, xs.len().min(ys.len()), 1)?.out)
    }

    /// `n` may be longer than the position slices. Those rows come back as error 1.
    pub fn seek_n(
        &self,
        xs: &[u32],
        ys: &[u32],
        txs: &[u32],
        tys: &[u32],
        step: u32,
        mask: &[u8],
        width: u32,
        height: u32,
        n: usize,
    ) -> Result<SeekOut, GpuError> {
        Ok(self.seek_repeat(xs, ys, txs, tys, step, mask, width, height, n, 1)?.out)
    }

    /// `rounds` greedy steps. Each round's landing positions are the next round's agents.
    /// Block size is 128, matching the h5i-app-gpu launch shape.
    pub fn seek_repeat(
        &self,
        xs: &[u32],
        ys: &[u32],
        txs: &[u32],
        tys: &[u32],
        step: u32,
        mask: &[u8],
        width: u32,
        height: u32,
        n: usize,
        rounds: u32,
    ) -> Result<SeekReport, GpuError> {
        if n > MAX_AGENTS {
            return Err(GpuError { op: "capacity", code: -2 });
        }
        if n == 0 || rounds == 0 {
            return Ok(SeekReport {
                out: SeekOut { xs: Vec::new(), ys: Vec::new(), errors: Vec::new() },
                kernel_ms: 0.0,
                wall_ms: 0.0,
            });
        }
        let wall = Instant::now();
        check("cuCtxSetCurrent", unsafe { cuCtxSetCurrent(self.ctx) })?;
        let mut bufs = self.workspace.borrow_mut();
        bufs.rewind();
        // Repeat copies `n` outputs back over the positions, so the buffers
        // have to cover a short slice whose `n` is longer than the input.
        let d_xs = bufs.alloc(xs.len().max(n) * 4)?;
        let d_ys = bufs.alloc(ys.len().max(n) * 4)?;
        let d_tx = bufs.alloc(txs.len() * 4)?;
        let d_ty = bufs.alloc(tys.len() * 4)?;
        let d_mask = bufs.alloc(mask.len())?;
        let d_out_x = bufs.alloc(n * 4)?;
        let d_out_y = bufs.alloc(n * 4)?;
        let d_err = bufs.alloc(n * 4)?;
        h2d(d_xs, xs)?;
        h2d(d_ys, ys)?;
        h2d(d_tx, txs)?;
        h2d(d_ty, tys)?;
        h2d(d_mask, mask)?;

        let mut start_ev: CUevent = std::ptr::null_mut();
        let mut end_ev: CUevent = std::ptr::null_mut();
        check("cuEventCreate", unsafe { cuEventCreate(&mut start_ev, 0) })?;
        let end_ok = unsafe { cuEventCreate(&mut end_ev, 0) };
        if end_ok != 0 {
            unsafe { let _ = cuEventDestroy_v2(start_ev); }
            return Err(GpuError { op: "cuEventCreate", code: end_ok });
        }

        let xs_len = xs.len() as u64;
        let ys_len = ys.len() as u64;
        let txs_len = txs.len() as u64;
        let tys_len = tys.len() as u64;
        let mask_len = mask.len() as u64;
        let step_arg = step;
        let width_arg = width;
        let height_arg = height;
        let n_arg = n as u64;
        let block = 128u32;
        let grid = ((n as u32) + block - 1) / block;
        let mut kernel_ms = 0f32;
        let mut round = 0u32;
        let launch_result = (|| {
            while round < rounds {
                check("cuEventRecord", unsafe { cuEventRecord(start_ev, std::ptr::null_mut()) })?;
                let mut args = [
                    &d_xs as *const CUdeviceptr as *mut c_void,
                    &xs_len as *const u64 as *mut c_void,
                    &d_ys as *const CUdeviceptr as *mut c_void,
                    &ys_len as *const u64 as *mut c_void,
                    &d_tx as *const CUdeviceptr as *mut c_void,
                    &txs_len as *const u64 as *mut c_void,
                    &d_ty as *const CUdeviceptr as *mut c_void,
                    &tys_len as *const u64 as *mut c_void,
                    &d_mask as *const CUdeviceptr as *mut c_void,
                    &mask_len as *const u64 as *mut c_void,
                    &step_arg as *const u32 as *mut c_void,
                    &width_arg as *const u32 as *mut c_void,
                    &height_arg as *const u32 as *mut c_void,
                    &d_out_x as *const CUdeviceptr as *mut c_void,
                    &d_out_y as *const CUdeviceptr as *mut c_void,
                    &d_err as *const CUdeviceptr as *mut c_void,
                    &n_arg as *const u64 as *mut c_void,
                ];
                check("cuLaunchKernel", unsafe {
                    cuLaunchKernel(
                        self.func,
                        grid,
                        1,
                        1,
                        block,
                        1,
                        1,
                        0,
                        std::ptr::null_mut(),
                        args.as_mut_ptr(),
                        std::ptr::null_mut(),
                    )
                })?;
                check("cuEventRecord", unsafe { cuEventRecord(end_ev, std::ptr::null_mut()) })?;
                check("cuEventSynchronize", unsafe { cuEventSynchronize(end_ev) })?;
                let mut ms = 0f32;
                check("cuEventElapsedTime", unsafe { cuEventElapsedTime(&mut ms, start_ev, end_ev) })?;
                kernel_ms += ms;
                let bytes = n * 4;
                check("cuMemcpyDtoD x", unsafe { cuMemcpyDtoD_v2(d_xs, d_out_x, bytes) })?;
                check("cuMemcpyDtoD y", unsafe { cuMemcpyDtoD_v2(d_ys, d_out_y, bytes) })?;
                round += 1;
            }
            Ok(())
        })();
        unsafe {
            let _ = cuEventDestroy_v2(start_ev);
            let _ = cuEventDestroy_v2(end_ev);
        }
        launch_result?;

        let mut out_x = vec![0u32; n];
        let mut out_y = vec![0u32; n];
        let mut errors = vec![0u32; n];
        let bytes = n * 4;
        check("cuMemcpyDtoH x", unsafe { cuMemcpyDtoH_v2(out_x.as_mut_ptr().cast(), d_xs, bytes) })?;
        check("cuMemcpyDtoH y", unsafe { cuMemcpyDtoH_v2(out_y.as_mut_ptr().cast(), d_ys, bytes) })?;
        check("cuMemcpyDtoH err", unsafe { cuMemcpyDtoH_v2(errors.as_mut_ptr().cast(), d_err, bytes) })?;
        Ok(SeekReport {
            out: SeekOut { xs: out_x, ys: out_y, errors },
            kernel_ms,
            wall_ms: wall.elapsed().as_secs_f64() * 1000.0,
        })
    }

    /// One sequential tick. The slices of positions, flags, and candidates must
    /// share a length. `head` and `next` are the start-of-tick spatial grid.
    pub fn step(
        &self,
        xs: &[u32],
        ys: &[u32],
        moving: &[u32],
        cand_x: &[u32],
        cand_y: &[u32],
        mask: &[u8],
        width: u32,
        height: u32,
        head: &[u32],
        next: &[u32],
    ) -> Result<StepReport, GpuError> {
        let n = xs.len();
        if ys.len() != n || moving.len() != n || cand_x.len() != n || cand_y.len() != n {
            return Err(GpuError { op: "shape", code: -3 });
        }
        if n > MAX_AGENTS {
            return Err(GpuError { op: "capacity", code: -2 });
        }
        if n == 0 {
            return Ok(StepReport {
                out: StepOut { xs: Vec::new(), ys: Vec::new(), error: 0 },
                kernel_ms: 0.0,
                wall_ms: 0.0,
            });
        }
        let wall = Instant::now();
        check("cuCtxSetCurrent", unsafe { cuCtxSetCurrent(self.ctx) })?;
        let mut bufs = self.workspace.borrow_mut();
        bufs.rewind();
        let d_xs = bufs.alloc(n * 4)?;
        let d_ys = bufs.alloc(n * 4)?;
        let d_moving = bufs.alloc(n * 4)?;
        let d_cx = bufs.alloc(n * 4)?;
        let d_cy = bufs.alloc(n * 4)?;
        let d_mask = bufs.alloc(mask.len())?;
        let d_head = bufs.alloc(head.len() * 4)?;
        let d_next = bufs.alloc(next.len() * 4)?;
        let d_err = bufs.alloc(4)?;
        h2d(d_xs, xs)?;
        h2d(d_ys, ys)?;
        h2d(d_moving, moving)?;
        h2d(d_cx, cand_x)?;
        h2d(d_cy, cand_y)?;
        h2d(d_mask, mask)?;
        h2d(d_head, head)?;
        h2d(d_next, next)?;
        let zero = 0u32;
        check("cuMemcpyHtoD err", unsafe { cuMemcpyHtoD_v2(d_err, &zero as *const u32 as *const c_void, 4) })?;

        let mut start_ev: CUevent = std::ptr::null_mut();
        let mut end_ev: CUevent = std::ptr::null_mut();
        check("cuEventCreate", unsafe { cuEventCreate(&mut start_ev, 0) })?;
        let end_ok = unsafe { cuEventCreate(&mut end_ev, 0) };
        if end_ok != 0 {
            unsafe { let _ = cuEventDestroy_v2(start_ev); }
            return Err(GpuError { op: "cuEventCreate", code: end_ok });
        }

        let xs_len = n as u64;
        let ys_len = n as u64;
        let moving_len = n as u64;
        let cx_len = n as u64;
        let cy_len = n as u64;
        let mask_len = mask.len() as u64;
        let head_len = head.len() as u64;
        let next_len = next.len() as u64;
        let width_arg = width;
        let height_arg = height;
        let n_arg = n as u64;
        let launch_result = (|| {
            check("cuEventRecord", unsafe { cuEventRecord(start_ev, std::ptr::null_mut()) })?;
            let mut args = [
                &d_xs as *const CUdeviceptr as *mut c_void,
                &xs_len as *const u64 as *mut c_void,
                &d_ys as *const CUdeviceptr as *mut c_void,
                &ys_len as *const u64 as *mut c_void,
                &d_moving as *const CUdeviceptr as *mut c_void,
                &moving_len as *const u64 as *mut c_void,
                &d_cx as *const CUdeviceptr as *mut c_void,
                &cx_len as *const u64 as *mut c_void,
                &d_cy as *const CUdeviceptr as *mut c_void,
                &cy_len as *const u64 as *mut c_void,
                &d_mask as *const CUdeviceptr as *mut c_void,
                &mask_len as *const u64 as *mut c_void,
                &d_head as *const CUdeviceptr as *mut c_void,
                &head_len as *const u64 as *mut c_void,
                &d_next as *const CUdeviceptr as *mut c_void,
                &next_len as *const u64 as *mut c_void,
                &width_arg as *const u32 as *mut c_void,
                &height_arg as *const u32 as *mut c_void,
                &d_err as *const CUdeviceptr as *mut c_void,
                &n_arg as *const u64 as *mut c_void,
            ];
            check("cuLaunchKernel", unsafe {
                cuLaunchKernel(
                    self.step_func,
                    1,
                    1,
                    1,
                    1,
                    1,
                    1,
                    0,
                    std::ptr::null_mut(),
                    args.as_mut_ptr(),
                    std::ptr::null_mut(),
                )
            })?;
            check("cuEventRecord", unsafe { cuEventRecord(end_ev, std::ptr::null_mut()) })?;
            check("cuEventSynchronize", unsafe { cuEventSynchronize(end_ev) })?;
            let mut ms = 0f32;
            check("cuEventElapsedTime", unsafe { cuEventElapsedTime(&mut ms, start_ev, end_ev) })?;
            Ok(ms)
        })();
        unsafe {
            let _ = cuEventDestroy_v2(start_ev);
            let _ = cuEventDestroy_v2(end_ev);
        }
        let kernel_ms = launch_result?;
        let mut out_x = vec![0u32; n];
        let mut out_y = vec![0u32; n];
        let mut error = 0u32;
        check("cuMemcpyDtoH x", unsafe { cuMemcpyDtoH_v2(out_x.as_mut_ptr().cast(), d_xs, n * 4) })?;
        check("cuMemcpyDtoH y", unsafe { cuMemcpyDtoH_v2(out_y.as_mut_ptr().cast(), d_ys, n * 4) })?;
        check("cuMemcpyDtoH err", unsafe { cuMemcpyDtoH_v2((&mut error) as *mut u32 as *mut c_void, d_err, 4) })?;
        Ok(StepReport {
            out: StepOut { xs: out_x, ys: out_y, error },
            kernel_ms,
            wall_ms: wall.elapsed().as_secs_f64() * 1000.0,
        })
    }

    /// One two-phase round. `cand_on` is all ones: every candidate is explicit.
    /// Positions match `tick_round` when `ids` are the player ids.
    pub fn round(
        &self,
        xs: &[u32],
        ys: &[u32],
        moving: &[u32],
        cand_x: &[u32],
        cand_y: &[u32],
        ids: &[u64],
        mask: &[u8],
        width: u32,
        height: u32,
    ) -> Result<StepReport, GpuError> {
        let n = xs.len();
        let cand_on = vec![1u32; n];
        self.rounds(xs, ys, moving, cand_x, cand_y, &cand_on, ids, mask, width, height, 1)
    }

    /// `rounds` two-phase steps. Candidates are round-major: round `r`, agent `i`
    /// lives at `r * n + i`. A zero `cand_on` entry means "use the position at
    /// the start of that round". Buffers stay on the device. The grid is rebuilt
    /// there between propose and resolve, and again at the start of the next round.
    /// Kernel time covers that device work. Wall time adds one copy in and one copy out.
    pub fn rounds(
        &self,
        xs: &[u32],
        ys: &[u32],
        moving: &[u32],
        cand_x: &[u32],
        cand_y: &[u32],
        cand_on: &[u32],
        ids: &[u64],
        mask: &[u8],
        width: u32,
        height: u32,
        rounds: u32,
    ) -> Result<StepReport, GpuError> {
        let n = xs.len();
        if ys.len() != n || moving.len() != n || ids.len() != n {
            return Err(GpuError { op: "shape", code: -3 });
        }
        if n > MAX_AGENTS {
            return Err(GpuError { op: "capacity", code: -2 });
        }
        if width == 0 || height == 0 {
            return Err(GpuError { op: "shape", code: -3 });
        }
        let cells = (width as usize).checked_mul(height as usize).ok_or(GpuError { op: "shape", code: -3 })?;
        if cells == 0 {
            return Err(GpuError { op: "shape", code: -3 });
        }
        if n == 0 || rounds == 0 {
            return Ok(StepReport {
                out: StepOut { xs: Vec::new(), ys: Vec::new(), error: 0 },
                kernel_ms: 0.0,
                wall_ms: 0.0,
            });
        }
        let rows = (rounds as usize).checked_mul(n).ok_or(GpuError { op: "shape", code: -3 })?;
        if cand_x.len() != rows || cand_y.len() != rows || cand_on.len() != rows {
            return Err(GpuError { op: "shape", code: -3 });
        }
        let wall = Instant::now();
        check("cuCtxSetCurrent", unsafe { cuCtxSetCurrent(self.ctx) })?;
        let mut bufs = self.workspace.borrow_mut();
        bufs.rewind();
        let d_xs = bufs.alloc(n * 4)?;
        let d_ys = bufs.alloc(n * 4)?;
        let d_moving = bufs.alloc(n * 4)?;
        let d_cx = bufs.alloc(rows * 4)?;
        let d_cy = bufs.alloc(rows * 4)?;
        let d_on = bufs.alloc(rows * 4)?;
        let d_ids = bufs.alloc(n * 8)?;
        let d_mask = bufs.alloc(mask.len())?;
        let d_head = bufs.alloc(cells * 4)?;
        let d_next = bufs.alloc(n * 4)?;
        let d_px = bufs.alloc(n * 4)?;
        let d_py = bufs.alloc(n * 4)?;
        let d_po = bufs.alloc(n * 4)?;
        h2d(d_xs, xs)?;
        h2d(d_ys, ys)?;
        h2d(d_moving, moving)?;
        h2d(d_cx, cand_x)?;
        h2d(d_cy, cand_y)?;
        h2d(d_on, cand_on)?;
        h2d(d_ids, ids)?;
        h2d(d_mask, mask)?;

        let mut start_ev: CUevent = std::ptr::null_mut();
        let mut end_ev: CUevent = std::ptr::null_mut();
        check("cuEventCreate", unsafe { cuEventCreate(&mut start_ev, 0) })?;
        let end_ok = unsafe { cuEventCreate(&mut end_ev, 0) };
        if end_ok != 0 {
            unsafe { let _ = cuEventDestroy_v2(start_ev); }
            return Err(GpuError { op: "cuEventCreate", code: end_ok });
        }

        let mask_len = mask.len() as u64;
        let width_arg = width;
        let height_arg = height;
        let n_arg = n as u64;
        let block = 128u32;
        let grid = ((n as u32) + block - 1) / block;
        let launch_result = (|| {
            check("cuEventRecord", unsafe { cuEventRecord(start_ev, std::ptr::null_mut()) })?;
            let mut round = 0u32;
            while round < rounds {
                let byte = (round as u64) * (n as u64) * 4;
                let cx = d_cx + byte;
                let cy = d_cy + byte;
                let on = d_on + byte;
                self.one_phase(d_xs, d_ys, d_moving, cx, cy, on, d_ids, d_mask, mask_len, d_head, d_next, d_px, d_py, d_po, width_arg, height_arg, 0, n_arg, cells, grid, block)?;
                if self.verify_grid {
                    check_grid(d_head, d_next, d_xs, d_ys, None, n, cells, width, height)?;
                }
                self.one_phase(d_xs, d_ys, d_moving, cx, cy, on, d_ids, d_mask, mask_len, d_head, d_next, d_px, d_py, d_po, width_arg, height_arg, 1, n_arg, cells, grid, block)?;
                self.one_phase(d_xs, d_ys, d_moving, cx, cy, on, d_ids, d_mask, mask_len, d_head, d_next, d_px, d_py, d_po, width_arg, height_arg, 2, n_arg, cells, grid, block)?;
                if self.verify_grid {
                    check_grid(d_head, d_next, d_px, d_py, Some(d_po), n, cells, width, height)?;
                }
                self.one_phase(d_xs, d_ys, d_moving, cx, cy, on, d_ids, d_mask, mask_len, d_head, d_next, d_px, d_py, d_po, width_arg, height_arg, 3, n_arg, cells, grid, block)?;
                round += 1;
            }
            check("cuEventRecord", unsafe { cuEventRecord(end_ev, std::ptr::null_mut()) })?;
            check("cuEventSynchronize", unsafe { cuEventSynchronize(end_ev) })?;
            let mut ms = 0f32;
            check("cuEventElapsedTime", unsafe { cuEventElapsedTime(&mut ms, start_ev, end_ev) })?;
            Ok(ms)
        })();
        unsafe {
            let _ = cuEventDestroy_v2(start_ev);
            let _ = cuEventDestroy_v2(end_ev);
        }
        let kernel_ms = launch_result?;
        let mut out_x = vec![0u32; n];
        let mut out_y = vec![0u32; n];
        check("cuMemcpyDtoH x", unsafe { cuMemcpyDtoH_v2(out_x.as_mut_ptr().cast(), d_xs, n * 4) })?;
        check("cuMemcpyDtoH y", unsafe { cuMemcpyDtoH_v2(out_y.as_mut_ptr().cast(), d_ys, n * 4) })?;
        Ok(StepReport {
            out: StepOut { xs: out_x, ys: out_y, error: 0 },
            kernel_ms,
            wall_ms: wall.elapsed().as_secs_f64() * 1000.0,
        })
    }

    /// Clear the chains, then launch one phase. Insert phases (0 and 2) need
    /// the clear. Propose and resolve do not, and skipping it keeps the lists
    /// built by the previous launch.
    fn one_phase(
        &self,
        d_xs: CUdeviceptr,
        d_ys: CUdeviceptr,
        d_moving: CUdeviceptr,
        d_cx: CUdeviceptr,
        d_cy: CUdeviceptr,
        d_on: CUdeviceptr,
        d_ids: CUdeviceptr,
        d_mask: CUdeviceptr,
        mask_len: u64,
        d_head: CUdeviceptr,
        d_next: CUdeviceptr,
        d_px: CUdeviceptr,
        d_py: CUdeviceptr,
        d_po: CUdeviceptr,
        width: u32,
        height: u32,
        phase: u32,
        n: u64,
        cells: usize,
        grid: u32,
        block: u32,
    ) -> Result<(), GpuError> {
        if phase == 0 || phase == 2 {
            check("cuMemsetD32 head", unsafe { cuMemsetD32_v2(d_head, u32::MAX, cells) })?;
            check("cuMemsetD32 next", unsafe { cuMemsetD32_v2(d_next, u32::MAX, n as usize) })?;
        }
        let phase_arg = phase;
        let width_arg = width;
        let height_arg = height;
        let n_arg = n;
        let mask_len_arg = mask_len;
        let mut args = [
            &d_xs as *const CUdeviceptr as *mut c_void,
            &d_ys as *const CUdeviceptr as *mut c_void,
            &d_moving as *const CUdeviceptr as *mut c_void,
            &d_cx as *const CUdeviceptr as *mut c_void,
            &d_cy as *const CUdeviceptr as *mut c_void,
            &d_on as *const CUdeviceptr as *mut c_void,
            &d_ids as *const CUdeviceptr as *mut c_void,
            &d_mask as *const CUdeviceptr as *mut c_void,
            &mask_len_arg as *const u64 as *mut c_void,
            &d_head as *const CUdeviceptr as *mut c_void,
            &d_next as *const CUdeviceptr as *mut c_void,
            &d_px as *const CUdeviceptr as *mut c_void,
            &d_py as *const CUdeviceptr as *mut c_void,
            &d_po as *const CUdeviceptr as *mut c_void,
            &width_arg as *const u32 as *mut c_void,
            &height_arg as *const u32 as *mut c_void,
            &phase_arg as *const u32 as *mut c_void,
            &n_arg as *const u64 as *mut c_void,
        ];
        check("cuLaunchKernel", unsafe {
            cuLaunchKernel(
                self.round_func,
                grid,
                1,
                1,
                block,
                1,
                1,
                0,
                std::ptr::null_mut(),
                args.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        })
    }
}

fn d2h<T: Copy + Default>(src: CUdeviceptr, len: usize) -> Result<Vec<T>, GpuError> {
    let mut out = vec![T::default(); len];
    if len > 0 {
        check("cuMemcpyDtoH", unsafe { cuMemcpyDtoH_v2(out.as_mut_ptr().cast(), src, std::mem::size_of_val(out.as_slice())) })?;
    }
    Ok(out)
}

/// Copy a device-built grid back and run the kernel's `grid_wellformed` on it.
/// `on` is `None` for the position grid, where every agent is inserted.
fn check_grid(
    d_head: CUdeviceptr,
    d_next: CUdeviceptr,
    d_xs: CUdeviceptr,
    d_ys: CUdeviceptr,
    d_on: Option<CUdeviceptr>,
    n: usize,
    cells: usize,
    width: u32,
    height: u32,
) -> Result<(), GpuError> {
    let head: Vec<u32> = d2h(d_head, cells)?;
    let next: Vec<u32> = d2h(d_next, n)?;
    let xs: Vec<u32> = d2h(d_xs, n)?;
    let ys: Vec<u32> = d2h(d_ys, n)?;
    let on: Vec<u32> = match d_on {
        Some(d) => d2h(d, n)?,
        None => vec![1; n],
    };
    let mut seen = vec![0u32; n];
    if ai_town_kernel::grid_wellformed(&head, &next, width, height, &xs, &ys, &on, &mut seen) {
        Ok(())
    } else {
        Err(GpuError { op: "grid_wellformed", code: -4 })
    }
}

impl Drop for RustDevice {
    fn drop(&mut self) {
        unsafe {
            let _ = cuCtxSetCurrent(self.ctx);
            self.workspace.get_mut().clear();
            let _ = cuModuleUnload(self.module);
            let _ = cuCtxDestroy_v2(self.ctx);
        }
    }
}
