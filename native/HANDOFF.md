# Handoff notes

Started in a claude.ai chat (2026-10-15), continued in Claude Code the same
day. Read this first when continuing the work. Follow CLAUDE.md: ask before
deciding anything, and explain concretely. Talk to the owner in Japanese;
files in the repo stay in English.

## 1. What the project is

**Scope (owner, 2026-10-15): Shuttle only provides a Weave-integrated,
persistable Wasmtime runtime and its Java binding**, the way JNLua provides
the Lua runtime for OpenComputers. It is **not a mod**. Planned consumers:

- **OpenComputers (OC)**, through a separate addon mod.
- **A new Retro Computing mod (RC)** by the owner, which includes Shuttle
  by default.

Neither integration exists yet. Everything at the level of "a computer"
belongs to the host mod: firmware and its ABI, devices, ticks, power,
BIOS -> OS -> child programs, the save format of a whole machine.

- Runtime: **Wasmtime**, made checkpointable with **Weave** (a Wasm
  transformer that can unwind a running program into its linear memory and
  rebuild it later). This is how a host mod saves running guests with the
  world and resumes them after a reload.
- Kept in Shuttle as generic runtime features (designed, not implemented):
  Weave extensions (yielding imports, `request_unwind`, memory limits) and
  the primitives for nested instances. See `docs/design/runtime-extensions.md`.
- `native/` (`shuttle-core`, `shuttle-runner`, C guests, firmware ABI) is a
  prototype of the RC side and will move to the RC repository.
- Native code: Rust (Wasmtime + Weave + JNI). Java side: `shuttle-java`.
- Targets: Windows, macOS, Linux, each x86_64 and arm64.

## 2. Where things are

| What | Where |
| --- | --- |
| This repo | `akkiserver-dev/shuttle` (forked from StefanRichterHuber/wasmtime-ng) |
| Working branch | `feature/unified-ci` (from `main` after PR #2 was merged as `7109d4f`) |
| PRs | #1 closed (CI check only); #2 merged (native core, rename, docs, CI fixes) |
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
Remote branches `feature/native-build` and `feature/shuttle-core` are merged/obsolete (not deleted yet).

## 3. Current state (2026-10-15)

- **Windows native build verified.** `native/scripts/build.ps1`: clang 23.1.3,
  rustc 1.99.0, `cargo build` 2m13s; verify 26/26 identical, control 24/24
  diverged. Re-run after the rebase: same result.
  - Snapshots on Windows are 5.6 KB (tick 1) and about 9.3 KB later; the
    Linux numbers are 5.2-7.7 KB (confirmed on WSL2 below).
  - `build.ps1` dies if its output is redirected (`*>&1`) under Windows
    PowerShell 5.1: cargo's stderr becomes an error record and
    `$ErrorActionPreference='Stop'` aborts. A plain run works. Not fixed.
- **WSL2 native build verified** (Ubuntu-26.04, run directly in
  `/mnt/d/Projects/shuttle/native`, `.gitattributes` keeps `*.sh` LF):
  `cargo build` 2m33s, verify 26/26 identical, control 24/24 diverged.
  Snapshots 5.3 KB (tick 1) and about 7.6 KB later, matching the Linux numbers
  in `native/README.md`; so the larger Windows snapshots (about 9.3 KB) are an
  OS difference, cause not investigated.
- **WSL2 Maven verified**: `mvn test` in `/mnt/d/Projects/shuttle` (no `-P`;
  `host-linux-x86-64` activates itself) passes for all five modules
  (4m07s). Needs `openjdk-21-jdk`, `maven` and the `wasm32-unknown-unknown`
  Rust target. The owner's bar for "done" (build + tests on the Windows host
  and on WSL2) is met.
- **CI (`ci.yml`, PR #3)**: all 13 jobs pass. One earlier `java
  (macos-aarch64)` failure was transient: Maven could not download
  `maven-filtering:3.4.0` from Central; a re-run passed. Further CI work is
  on hold by the owner's decision.
- **Maven on Windows** (IntelliJ's bundled Maven, JDK 25, no `-P`; the
  `host-windows-x86-64` profile activates itself), `RUSTFLAGS=-D warnings`:
  all five modules pass: `shuttle-java` 64/64, `shuttle-wit-parser` 4/4,
  `shuttle-wit-maven-plugin` 3/3.
- PR #2 (rename, docs, Java CI fixes) is merged; Java CI was green on Linux
  and Windows before the merge.
- On `feature/unified-ci`: `native.yml` + `maven.yml` replaced by one
  `ci.yml` (build only, six platforms); the cross-based Maven profiles
  replaced by six OS-activated `host-*` profiles in the root pom, also used by
  `shuttle-wit-parser`.
- **Two Weave versions coexist**: `shuttle-java` pins `0f26893` (no
  `shuttle-host-hooks`), `native/` pins `677cce6`. Decided: they are not
  connected; `shuttle-java` moves to `677cce6` as part of the
  runtime-extensions design, and `native/` moves to the RC side.

## 4. Decided with the owner

The first group was decided while Shuttle was still thought to include the
machine. Under the clarified scope (section 1) it belongs to the RC mod and
is kept here for reference until `native/` moves.

### 4a. Machine level (belongs to the RC mod)

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

### 4b. Shuttle (runtime and repository)

- Scope (2026-10-15): Shuttle = Weave-integrated persistable Wasmtime runtime
  + Java binding only. Kept as runtime features: Weave extensions (yielding
  imports, `request_unwind`, memory limits) and nested-instance primitives;
  design in `docs/design/runtime-extensions.md` (API names there still need
  approval). `native/` moves to the RC side.
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
- CI (2026-10-15): one workflow `ci.yml` with separate jobs (`guests`,
  `native` x6, `java` x6) on GitHub-hosted runners for all six platforms.
  **CI only builds**: no tests and no `shuttle-runner verify`. Tests run on
  the owner's Windows machine; if CI breaks on another platform, fix it
  separately. Triggers: push to `main`, every PR, manual dispatch. No
  per-path job filtering. Rust warnings are errors (`-D warnings`) in both.
- jni deprecation (jni-rs/jni-rs#846): `#![allow(deprecated)]` in the two
  JNI crates' `lib.rs` until jni ships a fix.
- Maven native build: six `host-*` profiles in the root pom, activated from
  `os.name`/`os.arch`, built with plain `cargo` (no cross, no Docker). Each
  jar holds the host platform's library only; an all-platform jar is a later
  task (distribution format not decided).

## 5. NOT approved yet (decided by Claude without asking; review each)

Item 3 concerns Shuttle (Weave API names). Items 1, 2, 4 and 5 concern the
machine prototype in `native/` and go to the RC side with it.

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

- No all-platform jar yet: each build packages only the host platform's
  native libraries.
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

The owner picks the order. Candidates:

1. Implement `docs/design/runtime-extensions.md` after the owner approves its
   API names (and section 5 item 3): Weave bump to `677cce6`, yielding
   imports, memory limits, nested-instance primitives, tests.
2. Move `native/` to the RC repository (when that repository exists).
3. All-platform jar / distribution (Maven Central, GitHub Releases, bundled
   in the RC mod, ...). CI work is on hold until the owner asks.
4. Smaller known issues from section 6.

Done on 2026-10-15: Windows and WSL2 builds + tests, rename to Shuttle,
docs, Java CI fix (PR #2), unified build-only CI and host profiles (PR #3),
scope clarified to runtime-only and the runtime-extensions design.
