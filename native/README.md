# Shuttle (prototype)

Native core of Shuttle, the Wasm runtime library for computer mods: runs Wasm guests (BIOS → OS kernel →
child programs) on Wasmtime 46, makes them checkpointable with Weave, and
implements the firmware ABI draft 0.1 (`fw.*`, `co.*`, `sys.call`).
Nothing here depends on Minecraft or Java yet.

## Layout

```
crates/shuttle-core     the engine: image cache, Machine, ABI host functions, snapshot format
crates/shuttle-runner   headless emulator (run / verify)
guests/shuttle.h        ABI header for C guests (freestanding wasm32)
guests/bios.c           finds a bootable disk, image_load, exec
guests/os.c             loads a program from the other disk, runs it as a child, serves sys.call
guests/child.c          user program: Hello, 5 rounds of deep-recursive work, sleep, exit(42)
```

Weave comes from `akkiserver-dev/weave`, branch `shuttle-host-hooks`
(commit `677cce6`), which adds two embedding hooks on top of the Wasmtime 46
update:

- **yielding imports** (`TransformOptions::yielding_imports` +
  `weave_wasmtime::request_unwind`): a host function can suspend the guest from
  inside the call; on resume the call runs again and returns the result.
- **memory limits** (`WeaveInstance::set_memory_limit`, `Ctx::set_memory_limit`):
  a Wasmtime `StoreLimits` limiter, so `memory.grow` past the cap returns -1.

## Build and run

Scripts (they build the guests, build the workspace and run `verify`):

```sh
./native/scripts/build.sh                    # Linux / WSL2: dev build + verify
./native/scripts/build.sh --release          # release build + verify
./native/scripts/build.sh --cross aarch64    # Linux arm64 via `cross` (build only)
```
```powershell
.\native\scripts\build.ps1                    # Windows: dev build + verify
.\native\scripts\build.ps1 -Release
```

Requirements: Rust (rustup); clang + wasm-ld (`sudo apt install clang lld`
on Linux/WSL2, the LLVM installer on Windows); `cross` + Docker only for
`--cross`.

By hand, inside `native/`:

```sh
sh guests/build.sh
cargo build
./target/debug/shuttle-runner run    guests/out/bios.wasm guests/out/os.wasm guests/out/child.wasm
./target/debug/shuttle-runner verify guests/out/bios.wasm guests/out/os.wasm guests/out/child.wasm
```

CI (`.github/workflows/native.yml`) builds the guests once on Linux, then
builds and runs `verify` on Linux, Windows and macOS, each on x86_64 and
arm64.

`run` boots and prints the log until power-off. `verify` runs once without
interruption, then once per tick boundary with capture → restore into a
brand-new Machine → continue, and compares the log streams. A control run
drops the host-side child table on restore and must diverge.

## Results (2026-10-15, Linux x86_64, 2 cores, dev profile)

| Check | Result |
| --- | --- |
| Boot chain BIOS → `exec` → OS → `co_create` → child | works, 27 ticks to power-off |
| Restore at every tick boundary (1–26) | 26/26 identical logs |
| Control: child table dropped on restore | 24/24 diverged (the check is real) |
| Snapshot size (whole machine, gzip) | 5.2–7.7 KB |
| Module without `shuttle.abi` | rejected with `E_VERSION` (-8) |
| SIMD module on a CPU without `simd` | rejected with `E_UNSUPPORTED` (-7) |
| `co_create` reserving more than the CPU's memory | `E_NOMEM` (-6) |
| Child `memory.grow` past its `mem_limit` | returns -1, child keeps running |
| `dev_call` result larger than `rcap` | size returned, result fetched once with `dev_result(-1)` |

Restore points cover the child in the middle of a 32-deep recursive
computation, the kernel halted in `hlt` (child sleeping), and a pending
`sys.call` waiting for its return value.

## Implemented vs. the ABI draft

Implemented: `cpuid` (string and numeric leaves), `hlt`, `yield`, `time`,
`log` (rate-limited), `shutdown`, `reboot`, `dev_slots`, `dev_info`,
`dev_describe` (empty), `dev_call`, `dev_submit`/`dev_result` (completed at
the next tick start, `COMPLETE` event), `poll_event` (with `OVERFLOW`),
`image_load` (ABI + feature check, cached by hash), `image_free`, `exec`,
`boot_args`, all `co_*`, `sys.call`.

Resolved since the first prototype:

1. **Exit-type calls are plain imports.** `hlt`, `yield`, `exec`,
   `shutdown`, `reboot` and `sys.call` are Weave yielding imports: the first
   call records the request and unwinds; on resume the call is executed again
   and returns the result (`fw.hlt` returns 1/0, `sys.call` returns the
   kernel's `retval`). Children import only `sys.call`, as the ABI says.
2. **Memory limits are enforced.** Each child's linear memory (including
   Weave's shadow stack) is capped at its `mem_limit`. The kernel may grow
   into whatever the children have not reserved; `co_create` fails with
   `E_NOMEM` if kernel memory + reservations + the new limit exceed the CPU.
3. **`dev_call` keeps results that do not fit.** It returns the needed size,
   and the result is held (one slot, replaced by the next `dev_call`) until
   it is fetched with `dev_result(-1, ret, rcap)`.

Other notes: `poll_event` reports `OVERFLOW` first when events were dropped;
queued `dev_submit` requests are saved in snapshots and run after restore.

## Next steps

- JNI layer in shuttle-java exposing `ShuttleEngine` / `Machine` (`tick`,
  `capture`, `restore`, `Device.invoke` callback).
- Measure with a real OS-sized module (transform/compile time, disk cache).
