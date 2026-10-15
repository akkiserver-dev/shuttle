#!/usr/bin/env bash
# Build and verify the Shuttle native workspace on Linux / WSL2.
#
#   ./native/scripts/build.sh                     dev build + verify
#   ./native/scripts/build.sh --release           release build + verify
#   ./native/scripts/build.sh --cross aarch64     build for Linux arm64 with `cross` (no verify)
#   ./native/scripts/build.sh --skip-verify       build only
#
# Needs: Rust (rustup), clang and lld (`sudo apt install clang lld`).
# `--cross` also needs `cargo install cross` and a running Docker.
set -euo pipefail

NATIVE="$(cd "$(dirname "$0")/.." && pwd)"
cd "$NATIVE"

PROFILE=dev
PROFILE_DIR=debug
CROSS_ARCH=""
VERIFY=1

while [ $# -gt 0 ]; do
    case "$1" in
        --release) PROFILE=release; PROFILE_DIR=release ;;
        --cross) CROSS_ARCH="${2:?--cross needs an architecture, e.g. aarch64}"; shift ;;
        --skip-verify) VERIFY=0 ;;
        -h|--help) sed -n '2,11p' "$0"; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

need() {
    command -v "$1" >/dev/null 2>&1 || { echo "error: '$1' not found. $2" >&2; exit 1; }
}

echo "== guests (C -> wasm32)"
need clang "Install it with: sudo apt install clang lld"
need wasm-ld "Install it with: sudo apt install lld"
sh guests/build.sh

if [ -n "$CROSS_ARCH" ]; then
    need cross "Install it with: cargo install cross (Docker must be running)"
    TARGET="${CROSS_ARCH}-unknown-linux-gnu"
    echo "== cross build for $TARGET ($PROFILE)"
    cross build --locked --profile "$PROFILE" --target "$TARGET"
    echo "built: target/$TARGET/$PROFILE_DIR/"
    echo "(verify is skipped for cross builds: the binary is for another CPU)"
    exit 0
fi

need cargo "Install Rust from https://rustup.rs"
echo "== cargo build ($PROFILE)"
cargo build --locked --profile "$PROFILE"

if [ "$VERIFY" = 1 ]; then
    echo "== verify (capture/restore at every tick)"
    "target/$PROFILE_DIR/shuttle-runner" verify \
        guests/out/bios.wasm guests/out/os.wasm guests/out/child.wasm
fi
