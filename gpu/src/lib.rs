//! Optional CUDA seek. The game tick stays on the CPU spatial grid in `kernel`.
//! This crate is outside extraction: Lean does not see it, and it does not
//! depend on h5i. `h5i-app-gpu` is not published on crates.io.
//!
//! [`Device::seek`] follows `ai_town_kernel::scalar_seek`. The equivalence
//! test is the check that the device matched the CPU oracle on the inputs it ran.

use std::ffi::c_void;
use std::marker::PhantomData;

const CUBIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/seek.cubin"));

#[derive(Debug)]
pub struct GpuError {
    pub op: &'static str,
    pub code: i32,
}

type CUresult = i32;
type CUdevice = i32;
type CUdeviceptr = u64;
type CUcontext = *mut c_void;
type CUmodule = *mut c_void;
type CUfunction = *mut c_void;

#[link(name = "cuda")]
unsafe extern "C" {
    fn cuInit(flags: u32) -> CUresult;
    fn cuDeviceGet(device: *mut CUdevice, ordinal: i32) -> CUresult;
    fn cuCtxCreate_v2(ctx: *mut CUcontext, flags: u32, dev: CUdevice) -> CUresult;
    fn cuCtxDestroy_v2(ctx: CUcontext) -> CUresult;
    fn cuModuleLoadData(module: *mut CUmodule, image: *const c_void) -> CUresult;
    fn cuModuleUnload(module: CUmodule) -> CUresult;
    fn cuModuleGetFunction(hfunc: *mut CUfunction, module: CUmodule, name: *const i8) -> CUresult;
    fn cuMemAlloc_v2(dptr: *mut CUdeviceptr, bytes: usize) -> CUresult;
    fn cuMemFree_v2(dptr: CUdeviceptr) -> CUresult;
    fn cuMemcpyHtoD_v2(dst: CUdeviceptr, src: *const c_void, bytes: usize) -> CUresult;
    fn cuMemcpyDtoH_v2(dst: *mut c_void, src: CUdeviceptr, bytes: usize) -> CUresult;
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
    fn cuCtxSynchronize() -> CUresult;
}

fn check(op: &'static str, code: CUresult) -> Result<(), GpuError> {
    if code == 0 {
        Ok(())
    } else {
        Err(GpuError { op, code })
    }
}

pub struct Device {
    ctx: CUcontext,
    module: CUmodule,
    func: CUfunction,
    _thread: PhantomData<*const ()>,
}

impl Device {
    /// GPU 0. The cubin is sm_86 unless `AI_TOWN_CUDA_ARCH` selected another arch at build time.
    pub fn open() -> Result<Self, GpuError> {
        unsafe {
            check("cuInit", cuInit(0))?;
            let mut dev: CUdevice = 0;
            check("cuDeviceGet", cuDeviceGet(&mut dev, 0))?;
            let mut ctx: CUcontext = std::ptr::null_mut();
            check("cuCtxCreate", cuCtxCreate_v2(&mut ctx, 0, dev))?;
            let mut module: CUmodule = std::ptr::null_mut();
            let loaded = cuModuleLoadData(&mut module, CUBIN.as_ptr().cast());
            if loaded != 0 {
                let _ = cuCtxDestroy_v2(ctx);
                return Err(GpuError { op: "cuModuleLoadData", code: loaded });
            }
            let mut func: CUfunction = std::ptr::null_mut();
            let name = c"seek_kernel";
            let got = cuModuleGetFunction(&mut func, module, name.as_ptr());
            if got != 0 {
                let _ = cuModuleUnload(module);
                let _ = cuCtxDestroy_v2(ctx);
                return Err(GpuError { op: "cuModuleGetFunction", code: got });
            }
            Ok(Device { ctx, module, func, _thread: PhantomData })
        }
    }

    /// One step, matching [`ai_town_kernel::scalar_seek`]. Targets shorter than
    /// the position lists are read as tile 0, which is what the CPU function does.
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
    ) -> Result<(Vec<u32>, Vec<u32>), GpuError> {
        let n = xs.len().min(ys.len());
        if n == 0 {
            return Ok((Vec::new(), Vec::new()));
        }
        if n > u32::MAX as usize {
            return Err(GpuError { op: "n", code: -1 });
        }
        let nu = n as u32;
        let mut tx = vec![0u32; n];
        let mut ty = vec![0u32; n];
        let mut i = 0;
        while i < n {
            if i < txs.len() {
                tx[i] = txs[i];
            }
            if i < tys.len() {
                ty[i] = tys[i];
            }
            i += 1;
        }
        let mask_len = u32::try_from(mask.len()).unwrap_or(u32::MAX);
        let mask_bytes = mask.len().min(mask_len as usize);
        unsafe {
            self.launch(
                &xs[..n],
                &ys[..n],
                &tx,
                &ty,
                step,
                &mask[..mask_bytes],
                mask_len.min(mask_bytes as u32),
                nu,
                width,
                height,
            )
        }
    }

    unsafe fn launch(
        &self,
        xs: &[u32],
        ys: &[u32],
        tx: &[u32],
        ty: &[u32],
        step: u32,
        mask: &[u8],
        mask_len: u32,
        n: u32,
        width: u32,
        height: u32,
    ) -> Result<(Vec<u32>, Vec<u32>), GpuError> {
        let count = n as usize;
        let bytes = count * 4;
        let mut d_xs = 0;
        let mut d_ys = 0;
        let mut d_tx = 0;
        let mut d_ty = 0;
        let mut d_ox = 0;
        let mut d_oy = 0;
        let mut d_mask = 0;
        let mask_bytes = mask.len().max(1);
        unsafe {
            check("cuMemAlloc", cuMemAlloc_v2(&mut d_xs, bytes))?;
            check("cuMemAlloc", cuMemAlloc_v2(&mut d_ys, bytes))?;
            check("cuMemAlloc", cuMemAlloc_v2(&mut d_tx, bytes))?;
            check("cuMemAlloc", cuMemAlloc_v2(&mut d_ty, bytes))?;
            check("cuMemAlloc", cuMemAlloc_v2(&mut d_ox, bytes))?;
            check("cuMemAlloc", cuMemAlloc_v2(&mut d_oy, bytes))?;
            check("cuMemAlloc", cuMemAlloc_v2(&mut d_mask, mask_bytes))?;
            let copy_in = self.copy_in(d_xs, d_ys, d_tx, d_ty, d_mask, xs, ys, tx, ty, mask);
            if let Err(err) = copy_in {
                self.free(d_xs, d_ys, d_tx, d_ty, d_ox, d_oy, d_mask);
                return Err(err);
            }
            let n_arg = n;
            let step_arg = step;
            let width_arg = width;
            let height_arg = height;
            let mask_len_arg = mask_len;
            let mut args = [
                &d_xs as *const CUdeviceptr as *mut c_void,
                &d_ys as *const CUdeviceptr as *mut c_void,
                &d_tx as *const CUdeviceptr as *mut c_void,
                &d_ty as *const CUdeviceptr as *mut c_void,
                &d_ox as *const CUdeviceptr as *mut c_void,
                &d_oy as *const CUdeviceptr as *mut c_void,
                &d_mask as *const CUdeviceptr as *mut c_void,
                &mask_len_arg as *const u32 as *mut c_void,
                &n_arg as *const u32 as *mut c_void,
                &step_arg as *const u32 as *mut c_void,
                &width_arg as *const u32 as *mut c_void,
                &height_arg as *const u32 as *mut c_void,
            ];
            let block = 256u32;
            let grid = (n + block - 1) / block;
            let launched = cuLaunchKernel(
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
            );
            if launched != 0 {
                self.free(d_xs, d_ys, d_tx, d_ty, d_ox, d_oy, d_mask);
                return Err(GpuError { op: "cuLaunchKernel", code: launched });
            }
            let synced = cuCtxSynchronize();
            if synced != 0 {
                self.free(d_xs, d_ys, d_tx, d_ty, d_ox, d_oy, d_mask);
                return Err(GpuError { op: "cuCtxSynchronize", code: synced });
            }
            let mut out_x = vec![0u32; count];
            let mut out_y = vec![0u32; count];
            let dx = cuMemcpyDtoH_v2(out_x.as_mut_ptr().cast(), d_ox, bytes);
            let dy = cuMemcpyDtoH_v2(out_y.as_mut_ptr().cast(), d_oy, bytes);
            self.free(d_xs, d_ys, d_tx, d_ty, d_ox, d_oy, d_mask);
            check("cuMemcpyDtoH x", dx)?;
            check("cuMemcpyDtoH y", dy)?;
            Ok((out_x, out_y))
        }
    }

    unsafe fn copy_in(
        &self,
        d_xs: CUdeviceptr,
        d_ys: CUdeviceptr,
        d_tx: CUdeviceptr,
        d_ty: CUdeviceptr,
        d_mask: CUdeviceptr,
        xs: &[u32],
        ys: &[u32],
        tx: &[u32],
        ty: &[u32],
        mask: &[u8],
    ) -> Result<(), GpuError> {
        let bytes = xs.len() * 4;
        unsafe {
            check("cuMemcpyHtoD xs", cuMemcpyHtoD_v2(d_xs, xs.as_ptr().cast(), bytes))?;
            check("cuMemcpyHtoD ys", cuMemcpyHtoD_v2(d_ys, ys.as_ptr().cast(), bytes))?;
            check("cuMemcpyHtoD tx", cuMemcpyHtoD_v2(d_tx, tx.as_ptr().cast(), bytes))?;
            check("cuMemcpyHtoD ty", cuMemcpyHtoD_v2(d_ty, ty.as_ptr().cast(), bytes))?;
            if mask.is_empty() {
                let zero = [0u8];
                check("cuMemcpyHtoD mask", cuMemcpyHtoD_v2(d_mask, zero.as_ptr().cast(), 1))?;
            } else {
                check("cuMemcpyHtoD mask", cuMemcpyHtoD_v2(d_mask, mask.as_ptr().cast(), mask.len()))?;
            }
        }
        Ok(())
    }

    unsafe fn free(
        &self,
        d_xs: CUdeviceptr,
        d_ys: CUdeviceptr,
        d_tx: CUdeviceptr,
        d_ty: CUdeviceptr,
        d_ox: CUdeviceptr,
        d_oy: CUdeviceptr,
        d_mask: CUdeviceptr,
    ) {
        unsafe {
            let _ = cuMemFree_v2(d_xs);
            let _ = cuMemFree_v2(d_ys);
            let _ = cuMemFree_v2(d_tx);
            let _ = cuMemFree_v2(d_ty);
            let _ = cuMemFree_v2(d_ox);
            let _ = cuMemFree_v2(d_oy);
            let _ = cuMemFree_v2(d_mask);
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            let _ = cuModuleUnload(self.module);
            let _ = cuCtxDestroy_v2(self.ctx);
        }
    }
}
