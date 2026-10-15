# Handoff notes

Started in a claude.ai chat (2026-10-15), continued in Claude Code the same
day. Read this first when continuing the work. Follow CLAUDE.md: ask before
deciding anything, and explain concretely. Talk to the owner in Japanese;
files in the repo stay in English.

## 1. What the project is

Shuttle is a **WebAssembly runtime library** (Java binding + native Rust)
for Minecraft computer mods. It is **not a mod itself**. Planned consumers:

- **OpenComputers (OC)**, through a separate addon mod.
- **A new Retro Computing mod (RC)** by the owner, which includes Shuttle
  by default.

Neither integration exists yet. Everything a computer runs is Wasm: a BIOS
(player-made, e.g. stored on an EEPROM item), an OS kernel, and the OS's
child programs.

- Runtime: **Wasmtime**, made checkpointable with **Weave** (a Wasm
  transformer that can unwind a running program into its linear memory and
  rebuild it later). This is how a host mod saves a computer with the world
  and resumes it mid-execution after a reload.
- Layers: firmware (implemented by Shuttle) -> BIOS -> OS -> child programs.
  The firmware talks to guests only through the firmware ABI, never through
  Wasmtime APIs.
- Native code: Rust (Wasmtime + Weave + JNI). Java side: `shuttle-java`.
- Targets: Windows, macOS, Linux, each x86_64 and arm64.

## 2. Where things are

| What | Where |
| --- | --- |
| This repo | `akkiserver-dev/shuttle` (forked from StefanRichterHuber/wasmtime-ng) |
| Working branch | `feature/native-build`, rebased onto `main` (on top of `d400a75`) |
| Draft PR (CI check only, not for merging yet) | `akkiserver-dev/shuttle` PR #1 |
| Weave fork | `akkiserver-dev/weave`, branch `shuttle-host-hooks` (commit `677cce6`) |
| Firmware ABI spec (Japanese) | https://claude.ai/code/artifact/05272d17-ac4c-4f56-933a-374281672f79 |
| Docs | `README.md` (overview), `docs/` (building, java-binding, wit-codegen), `native/README.md` |

Modules (Maven groupId `jp.akkiserver`, parent artifactId `shuttle`):

| Directory | Java package | What |
| --- | --- | --- |
| `native/` | - | Rust workspace: `shuttle-core`, `shuttle-runner`, C test guests |
| `shuttle-java/` | `jp.akkiserver.shuttle` | Wasmtime + Weave JNI binding (`wasmtime-binding` crate inside) |
| `shuttle-java-example/` | `example` | Weave example (tick slices, snapshot, restore) |
| `shuttle-wit-parser/` | `jp.akkiserver.shuttle.wit.parser` | JNI binding to the Rust wit-parser crate |
| `shuttle-wit-maven-plugin/` | `jp.akkiserver.shuttle.wit.codegen` | WIT -> Java interface generator |

Old names (before commit `983dc25`): `wasmtimejavang`, `wasmtimejavang-example`,
`wit-parser`, `wit-codegen-maven-plugin`, groupId `io.github.stefanrichterhuber`,
packages `io.github.stefanrichterhuber.{wasmtimejavang,witparser,witcodegen}`.
`feature/shuttle-core` on the remote is an older subset of `feature/native-build`.

## 3. Current state (2026-10-15)

- **Windows native build verified.** `native/scripts/build.ps1`: clang 23.1.3,
  rustc 1.99.0, `cargo build` 2m13s; verify 26/26 identical, control 24/24
  diverged. Re-run after the rebase: same result.
  - Snapshots on Windows are 5.6 KB (tick 1) and about 9.3 KB later; the
    Linux numbers in `native/README.md` are 5.2-7.7 KB. Not investigated.
  - `build.ps1` dies if its output is redirected (`*>&1`) under Windows
    PowerShell 5.1: cargo's stderr becomes an error record and
    `$ErrorActionPreference='Stop'` aborts. A plain run works. Not fixed.
- **WSL2 native build not run yet.** Ubuntu-26.04 has only git; it needs
  `sudo apt install clang lld build-essential` and rustup (owner runs sudo).
  Plan: clone the repo into the WSL home and build there, so Linux and
  Windows artifacts do not share `native/target/`.
- **Maven on Windows** (IntelliJ's bundled Maven, JDK 25,
  `-P build-host-windows-x86_64`): `shuttle-java` 64/64 tests pass (incl.
  WeaveTest 12). `shuttle-wit-parser` fails on Windows because its pom only
  copies the Linux native library; `shuttle-wit-maven-plugin` was therefore
  not tested locally.
- Rename to Shuttle, docs rewrite and a code-generator import fix are
  committed. Pushed up to `7dbc5b8`; later commits are local only.
- **Two Weave versions coexist**: `shuttle-java` pins `0f26893` (no
  `shuttle-host-hooks`), `native/` pins `677cce6`. How the `d400a75` Weave
  JNI layer and `native/shuttle-core` fit together is **not decided**.

## 4. Decided with the owner

- Rust/Java boundary: Weave, `co_*`, VM exits, memory pool and snapshot
  capture live in Rust. Device implementations live in Java and are called
  directly from Rust (JNI) on worker threads; world-touching work is queued
  to the server thread. Host mod code sees a high-level Machine API only.
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
- Branches: `feature/<name>` and `fix/<name>`.
- Names (2026-10-15): modules, groupId and packages as in section 2;
  `native/` keeps its name. WIT tooling stays as two modules.
- Docs layout: root README = overview; `docs/building.md`,
  `docs/java-binding.md`, `docs/wit-codegen.md`, `docs/README.md` (index).
  WASI material was dropped because WASI was removed upstream (`44c4363`).
- pom `<developers>`: akki (maintainer) + Stefan Richter-Huber (original
  author).

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

## 6. Known issues (found 2026-10-15, not fixed)

- **`.github/workflows/maven.yml`** (probably why "Java CI with Maven"
  fails): installs `wasm32-wasip1`/`wasip2`/nightly `wasip1-threads`
  (leftovers from removed WASI modules) but not `wasm32-unknown-unknown`,
  which the `valuetest` fixture needs; cross-compiles
  `x86_64-pc-windows-msvc` with `cross`, which has no official msvc image.
- `shuttle-wit-parser` has no Windows/macOS native library copy step.
- No macOS profile in `shuttle-java/pom.xml`.
- Generated WIT contexts default to version `0.2.6`
  (`WasmComponentContext.DEFAULT_VERSION`); a WIT package at `@1.0.0` is
  registered under the wrong version unless `getVersion()` is overridden.
- `wasmcomponentvalue.rs`: an empty `list<u8>` becomes a Java `List`, not
  `byte[]`; WIT `char` is narrowed with `as u16` (breaks code points above
  U+FFFF).
- `WeaveModule.entries()` can report `v`/`r` type chars that
  `Builder.define` rejects, although its javadoc says "as in define".
- Public API typo: `getMiniumVersion` (with `getMaximumVersion`).
- The `linkContext` dependency lookup lambda ignores the version.
- `shuttle-java-example`'s pom description and comments are in Japanese.

## 7. Next steps (ask before each)

See the end of the 2026-10-15 Claude Code session; the owner picks the
order. Candidates: WSL2 build, fixing `maven.yml`, reviewing section 5, and
deciding how the Weave JNI layer and `native/shuttle-core` relate (then
designing the Machine API JNI layer).
