#!/usr/bin/env bash
# Compile the two-phase movement and the seek baseline to PTX. Toolchain,
# LLVM, and the target dir stay under /dev/shm/ai-town-rust. Pass extra cargo
# args after -- .
set -euo pipefail
cd "$(dirname "$0")/device"
root=/dev/shm/ai-town-rust
export CARGO_HOME="$root/cargo-home"
export CARGO_TARGET_DIR="$root/nvvm-target"
export RUSTUP_HOME="$root/rustup"
export CUDA_PATH=/usr/local/cuda-12.6
export CUDA_VISIBLE_DEVICES=0
export TMPDIR="$root/tmp"
mkdir -p "$TMPDIR"
export PATH="/usr/local/cuda/bin:$PATH"
toolchain="$RUSTUP_HOME/toolchains/nightly-2026-04-02-x86_64-unknown-linux-gnu"
export LD_LIBRARY_PATH="/usr/local/cuda-12.6/nvvm/lib64:/usr/local/cuda-12.6/lib64:$toolchain/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export BINDGEN_EXTRA_CLANG_ARGS=-I/usr/lib/gcc/x86_64-linux-gnu/13/include
export RUSTFLAGS="-Zcodegen-backend=$CARGO_TARGET_DIR/release/librustc_codegen_nvvm.so -Zunstable-options -Zcrate-attr=feature(register_tool) -Zcrate-attr=register_tool(nvvm_internal) -Zcrate-attr=feature(asm_experimental_arch) -Zcrate-attr=no_std -Zsaturating_float_casts=false -Cpanic=immediate-abort -Cllvm-args=-arch=compute_86"
# The sources sit on NFS, whose clock can run seconds behind this host, so a
# file edited right after a build can look older than the PTX in /dev/shm and
# cargo would keep the stale image. Rebuild the device crate whenever the
# content of anything it includes changes.
stamp="$CARGO_TARGET_DIR/ai-town-device-src.sha256"
sum=$(cat src/lib.rs Cargo.toml ../../src/layout.rs ../../../kernel/src/*.rs | sha256sum | cut -d' ' -f1)
if [[ ! -f "$stamp" || "$(cat "$stamp")" != "$sum" ]]; then
  find "$CARGO_TARGET_DIR/nvptx64-nvidia-cuda" -maxdepth 3 -name '*ai_town_seek_device*' -delete 2>/dev/null || true
  rm -rf "$CARGO_TARGET_DIR"/nvptx64-nvidia-cuda/release/.fingerprint/ai-town-seek-device-*
fi
lock=()
if [[ -f Cargo.lock ]]; then
  lock=(--locked)
fi
cargo +nightly-2026-04-02 build --release --manifest-path Cargo.toml --features gpu --target nvptx64-nvidia-cuda -Zbuild-std=core,alloc -j 8 "${lock[@]}" "$@"
echo "$sum" > "$stamp"
