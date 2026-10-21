#!/usr/bin/env bash
# Rebuild the pinned rustc_codegen_nvvm. Outputs stay under /dev/shm/ai-town-rust.
# The tree at that path is already populated; run this only to recreate it.
set -euo pipefail
root=/dev/shm/ai-town-rust
export CARGO_HOME="$root/cargo-home"
export CARGO_TARGET_DIR="$root/nvvm-target"
export RUSTUP_HOME="$root/rustup"
export TMPDIR="$root/tmp"
mkdir -p "$TMPDIR" "$CARGO_HOME" "$RUSTUP_HOME"
export CUDA_PATH=/usr/local/cuda-12.6
export CUDA_VISIBLE_DEVICES=0
export PATH="/usr/local/cuda/bin:$PATH"
export LD_LIBRARY_PATH="/usr/local/cuda-12.6/nvvm/lib64:/usr/local/cuda-12.6/lib64${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export BINDGEN_EXTRA_CLANG_ARGS=-I/usr/lib/gcc/x86_64-linux-gnu/13/include
source_dir="$root/rust-cuda"
rev=3a82c35a7196731fb60eebdac2a40d2b1bf0ba95
if [[ ! -d $source_dir/.git ]]; then
  git clone https://github.com/Rust-GPU/rust-cuda "$source_dir"
fi
git -C "$source_dir" checkout --detach "$rev"
rustup toolchain install --no-self-update nightly-2026-04-02 --profile minimal --component rust-src,rustc-dev,llvm-tools-preview
libs="$root/system-libs"
mkdir -p "$libs"
if ! compgen -G "$libs/*.deb" > /dev/null && [[ ! -d $libs/usr ]]; then
  (
    cd "$libs"
    apt-get download libssl-dev libssl3t64 libncurses-dev libxml2-dev libxml2 libtinfo6
    for package in *.deb; do dpkg-deb -x "$package" "$libs"; done
  )
fi
export OPENSSL_DIR="$libs/usr"
export OPENSSL_LIB_DIR="$libs/usr/lib/x86_64-linux-gnu"
export CFLAGS="-I$libs/usr/include/x86_64-linux-gnu"
export LIBRARY_PATH="$libs/usr/lib/x86_64-linux-gnu"
cd "$source_dir"
USE_PREBUILT_LLVM=1 cargo +nightly-2026-04-02 build --release --locked -p rustc_codegen_nvvm -j 8
