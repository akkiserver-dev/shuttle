# Handoff notes (from the claude.ai chat, 2026-10-15)

Read this first when continuing the work in Claude Code. Follow CLAUDE.md:
ask before deciding anything, and explain concretely.

## 1. What the project is

A Minecraft computer mod (like OpenComputers / CC:Tweaked) whose computers
run **WebAssembly** programs. Everything the player runs is Wasm: a BIOS
(stored in an EEPROM item), an OS kernel, and the OS's child programs.

- Runtime: **Wasmtime**, made checkpointable with **Weave** (a Wasm
  transformer that can unwind a running program into its linear memory and
  rebuild it later). This is how a computer is saved with the world and
  resumes mid-execution after a reload.
- Layers: firmware (provided by the mod) -> BIOS (player-made) -> OS
  (player-made) -> child programs. The firmware talks to guests only
  through the firmware ABI, never through Wasmtime APIs.
- Native code: Rust (Wasmtime + Weave + JNI). Mod side: Java (NeoForge).
- Targets: Windows, macOS, Linux, each x86_64 and arm64.

## 2. Where things are

| What | Where |
| --- | --- |
| This repo | `akkiserver-dev/shuttle` (renamed from `wasmtime-ng`), Java Wasmtime binding + `native/` |
| Weave fork | `akkiserver-dev/weave`, branch `shuttle-host-hooks` (commit `677cce6`) |
| Shuttle core prototype | branch `feature/shuttle-core` (`native/`) |
| Build scripts + CI | branch `feature/native-build` (on top of `feature/shuttle-core`) |
| Draft PR (CI check only, not for merging) | `akkiserver-dev/shuttle` PR #1 |
| Firmware ABI spec (Japanese) | https://claude.ai/code/artifact/05272d17-ac4c-4f56-933a-374281672f79 |

`native/README.md` explains the prototype, how to build it, and the results.

## 3. Current state

- `native/` boots BIOS -> `exec` -> OS -> `co_create` -> child -> exit, in a
  headless runner. Capture/restore at every tick boundary gives identical
  logs (26/26), and a control run that drops the host-side child table
  diverges (24/24), so the check is meaningful. Snapshots are 5-8 KB gzip.
- CI (`.github/workflows/native.yml`) passed on all six platforms on PR #1.
- `native/scripts/build.ps1` (Windows) has **never been run**. Test it first.
- "Java CI with Maven" fails, also on `main` itself (commit `d400a75`), so it
  is not caused by `native/`. The log could not be read from the chat
  sandbox; check it locally.
- **`main` moved on**: commit `d400a75 "Weave JNI"` (by the owner) added a
  Weave JNI layer in `wasmtimejavang` (`WeaveModule`, `WeaveInstance`,
  `weave.rs`, ...) pinned to Weave `0f26893`, i.e. *without* the
  `shuttle-host-hooks` patch. The feature branches are based on `917ffd6`,
  before that commit. How this JNI layer and `native/shuttle-core` fit
  together has **not been discussed yet**.

## 4. Decided with the owner

- Rust/Java boundary: Weave, `co_*`, VM exits, memory pool and snapshot
  capture live in Rust. Device implementations live in Java and are called
  directly from Rust (JNI) on worker threads; world-touching work is queued
  to the server thread. Mod code sees a high-level Machine API only.
- Threads: Rust owns a worker pool (reliable stack sizes for
  `max_wasm_stack`). Saving is split: capture on the worker at a tick
  boundary, compress and write on a separate save thread.
- Persistence: NBT/DataComponents hold only a UUID and small metadata; the
  snapshot and file systems are separate files in the world folder.
- ABI shape (see the spec): balanced realism. Kept: `cpuid` strings, slots
  on a bus, `hlt`, `exec`, `co_*` VM exits. Dropped: register I/O, manual
  DMA rings. Device calls are OC-like method calls, sync (`dev_call`) and
  async (`dev_submit` + completion event). Interrupts are polled.
- `cpuid` vendor and model names are set by the player in the assembler;
  other CPU properties come from parts. Feature flags map to Wasm features.
- Runtimes are switchable through an OC-style Architecture abstraction. Lua
  comes later (Wasm-hosted Lua preferred; JNLua as a compatibility option).
- Build: option C. Local scripts for Windows and WSL2 build the host target;
  release binaries for all six platforms come from CI.
- Weave may be modified freely.
- Branches: `feature/<name>` and `fix/<name>`. CLAUDE.md lives on `main`.

## 5. NOT approved yet (decided by Claude without asking; review each)

1. `dev_call` result that does not fit `rcap`: held in one slot and fetched
   with `dev_result(-1, ...)`; replaced by the next `dev_call`.
2. Kernel memory: the kernel may use whatever its children have not
   reserved; `co_create` fails with `E_NOMEM` when kernel memory +
   reservations + the new limit exceed the CPU's memory.
3. Weave API names: `TransformOptions::yielding_imports`,
   `weave_wasmtime::request_unwind`, `WeaveInstance::set_memory_limit`,
   `Ctx::set_memory_limit`; branch name `shuttle-host-hooks`.
4. Prototype values: `poll_period` 64, `stack_pages` 4, default CPU
   (clock 2000 polls/tick, 64 MiB), event queue 256, `exec` args 4 KiB,
   16 async requests, log 64 lines/tick, the guests' syscall numbers, and
   the snapshot format (`SHTL`, format 2).
5. `native/` layout details (`crates/`, `guests/`, `scripts/`), and that
   built `.wasm` files are not committed.

## 6. Next steps (in order, ask before each)

1. Run `native/scripts/build.ps1` on Windows and `build.sh` on WSL2.
2. Look at why "Java CI with Maven" fails on `main`.
3. Review the unapproved items in section 5.
4. Decide how the `d400a75` Weave JNI layer and `native/shuttle-core`
   relate, then design the JNI layer for the Machine API.
5. Rebase the feature branches onto the current `main`.
