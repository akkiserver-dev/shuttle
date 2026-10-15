[![Native (Shuttle)](https://github.com/akkiserver-dev/shuttle/actions/workflows/native.yml/badge.svg)](https://github.com/akkiserver-dev/shuttle/actions/workflows/native.yml)

# Shuttle

> **Status:** early prototype. The native core runs headless only; no mod uses Shuttle yet.

Shuttle is a WebAssembly runtime library for Minecraft computer mods. It is not a mod itself: computer mods use it to run the programs on their in-game computers. Two consumers are planned: OpenComputers (OC), through a separate addon mod, and a new Retro Computing mod, which will include Shuttle by default. Neither integration exists yet.

On behalf of the host mod, Shuttle runs a stack of Wasm guests:

```
firmware          implemented by Shuttle's native core
  └─ BIOS            player-made Wasm (for example stored on an EEPROM item)
       └─ OS kernel  player-made Wasm, started by the BIOS with `exec`
            └─ child programs  started by the kernel with `co_create`
```

The firmware talks to guests only through the **firmware ABI** (`fw.*`, `co.*`, `sys.call`). Device calls look like OpenComputers method calls, either synchronous (`dev_call`) or asynchronous (`dev_submit` + a completion event). The ABI is specified in a separate draft document (draft 0.1, not published in this repository); [`native/README.md`](native/README.md) lists which parts of it are implemented.

## How it works

- **Runtime:** [Wasmtime](https://github.com/bytecodealliance/wasmtime), made checkpointable with [Weave](https://github.com/akkiserver-dev/weave) (branch `shuttle-host-hooks`). Weave transforms a Wasm module so that a running program can be unwound into its linear memory and rebuilt later. This lets a host mod save a computer with the world and continue it mid-execution after a reload.
- **Native core (Rust, `native/`):** Wasmtime, Weave, the firmware ABI, the `co_*` machinery, memory limits and snapshot capture. Today it is driven by the headless `shuttle-runner`.
- **Java binding (`shuttle-java`):** Java API over Wasmtime and Weave through JNI, for the host mod to call. A high-level Machine API on top of the native core is planned but not written yet.
- **Targets:** Windows, macOS and Linux, each on x86_64 and arm64.

## Repository layout

| Path | What |
| --- | --- |
| [`native/`](native) | Shuttle native core (Rust): `shuttle-core`, the headless `shuttle-runner`, and C test guests in `native/guests`. See [`native/README.md`](native/README.md). |
| [`shuttle-java/`](shuttle-java) | Java bindings for Wasmtime plus the Weave (checkpointable Wasm) JNI layer. See [docs/java-binding.md](docs/java-binding.md). |
| [`shuttle-java-example/`](shuttle-java-example) | Runnable Weave example: run a guest a few "ticks" at a time, snapshot it, restore it and continue. |
| [`shuttle-wit-parser/`](shuttle-wit-parser), [`shuttle-wit-maven-plugin/`](shuttle-wit-maven-plugin) | Build-time WIT tooling: generate Java component-context interfaces from WIT files. See [docs/wit-codegen.md](docs/wit-codegen.md). |
| [`docs/`](docs) | Documentation. Start at [docs/README.md](docs/README.md). |

The Java modules form one Maven reactor: groupId `jp.akkiserver`, parent artifactId `shuttle`, one artifact per directory (`shuttle-java`, `shuttle-java-example`, `shuttle-wit-parser`, `shuttle-wit-maven-plugin`). The version is set in the root [`pom.xml`](pom.xml).

## Quick start

Native core (requires Rust, plus clang and wasm-ld for the C guests):

```powershell
.\native\scripts\build.ps1          # Windows: build guests + workspace, then verify
```
```sh
./native/scripts/build.sh           # Linux / WSL2
```

`verify` boots BIOS → OS → child, then repeats the run with capture → restore into a fresh machine at every tick boundary and checks that the logs are identical.

Java bindings on Windows (requires JDK 21, Maven and Rust; no Docker):

```powershell
mvn -P build-host-windows-x86_64 -pl shuttle-java,shuttle-java-example -am install
mvn -pl shuttle-java-example exec:java
```

The second command runs the Weave example. Linux builds, the other Maven profiles, CI and known problems are covered in [docs/building.md](docs/building.md).

## Documentation

- [docs/building.md](docs/building.md): building the native core and the Maven modules, CI, requirements, known issues
- [docs/java-binding.md](docs/java-binding.md): the `shuttle-java` API (Wasmtime and Weave) and its architecture
- [docs/wit-codegen.md](docs/wit-codegen.md): generating Java component contexts from WIT
- [native/README.md](native/README.md): the native core prototype, its results and its ABI coverage

## Credits and license

The Java Wasmtime bindings in this repository are a fork of [StefanRichterHuber/wasmtime-ng](https://github.com/StefanRichterHuber/wasmtime-ng) (MIT, © 2024 Stefan Richter-Huber). See [LICENSE](LICENSE). Upstream names these earlier Java bindings as prior work: [bluejekyll/wasmtime-java](https://github.com/bluejekyll/wasmtime-java) and [kawamuray/wasmtime-java](https://github.com/kawamuray/wasmtime-java).

> **Disclaimer:** This project is an independent community effort and is not affiliated with, maintained, or endorsed by the [Wasmtime project](https://github.com/bytecodealliance/wasmtime), the Bytecode Alliance, Mojang or Microsoft.

> **AI Disclaimer:** Parts of this project were vibe-coded or written with the help of an AI assistant (Claude); the remaining parts were written by hand.
