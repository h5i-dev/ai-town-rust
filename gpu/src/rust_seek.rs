//! Launch the PTX image of `kernel`'s `seek_one`.
//!
//! The handwritten cubin in `seek.cu` stays as a baseline. This launcher uses
//! the same driver API and does not depend on the `cust` crate. The CUDA
//! context is tied to the thread that opened it.

use crate::{GpuError, MAX_AGENTS};
use std::ffi::c_void;
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

pub struct RustDevice {
    ctx: CUcontext,
    module: CUmodule,
    func: CUfunction,
    _thread: PhantomData<*const ()>,
}

struct GpuBufs {
    ptrs: Vec<CUdeviceptr>,
}

impl GpuBufs {
    fn new() -> Self {
        Self { ptrs: Vec::new() }
    }

    fn alloc(&mut self, bytes: usize) -> Result<CUdeviceptr, GpuError> {
        let mut ptr = 0;
        check("cuMemAlloc", unsafe { cuMemAlloc_v2(&mut ptr, bytes.max(1)) })?;
        self.ptrs.push(ptr);
        Ok(ptr)
    }
}

impl Drop for GpuBufs {
    fn drop(&mut self) {
        for ptr in self.ptrs.drain(..) {
            unsafe {
                let _ = cuMemFree_v2(ptr);
            }
        }
    }
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
            Ok(RustDevice { ctx, module, func, _thread: PhantomData })
        }
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
        let mut bufs = GpuBufs::new();
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
}

impl Drop for RustDevice {
    fn drop(&mut self) {
        unsafe {
            let _ = cuCtxSetCurrent(self.ctx);
            let _ = cuModuleUnload(self.module);
            let _ = cuCtxDestroy_v2(self.ctx);
        }
    }
}
