#!/bin/sh
# Build the C guests into freestanding wasm32 modules.
set -e
cd "$(dirname "$0")"
mkdir -p out
for prog in bios os child; do
    clang --target=wasm32 -O2 -nostdlib -ffreestanding \
        -Wall -Wno-unused-function \
        -Wl,--no-entry -Wl,--export=_start -Wl,-z,stack-size=65536 \
        -o "out/$prog.wasm" "$prog.c"
done
ls -l out
