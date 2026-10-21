use std::process::Command;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    let cuda = std::env::var("CUDA_HOME").unwrap_or_else(|_| "/usr/local/cuda".to_string());
    let arch = std::env::var("AI_TOWN_CUDA_ARCH").unwrap_or_else(|_| "sm_86".to_string());
    let nvcc = format!("{cuda}/bin/nvcc");
    let src = format!("{manifest}/src/seek.cu");
    let dst = format!("{out}/seek.cubin");
    let status = Command::new(&nvcc)
        .args(["-cubin", "-O3", &format!("-arch={arch}"), &src, "-o", &dst])
        .status()
        .unwrap_or_else(|err| panic!("failed to run {nvcc}: {err}"));
    if !status.success() {
        panic!("nvcc failed to compile {src}");
    }
    println!("cargo:rerun-if-changed=src/seek.cu");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CUDA_HOME");
    println!("cargo:rerun-if-env-changed=AI_TOWN_CUDA_ARCH");
    println!("cargo:rustc-link-lib=dylib=cuda");
}
