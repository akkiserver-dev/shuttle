[![CI](https://github.com/akkiserver-dev/shuttle/actions/workflows/ci.yml/badge.svg)](https://github.com/akkiserver-dev/shuttle/actions/workflows/ci.yml)

# Shuttle

> **Status:** early prototype. No mod uses Shuttle yet.

Shuttle is a persistable WebAssembly runtime for Java: [Wasmtime](https://github.com/bytecodealliance/wasmtime) integrated with [Weave](https://github.com/akkiserver-dev/weave), plus its Java binding. It plays the role for Wasm that JNLua plays for Lua in OpenComputers: it runs guest code and can save it mid-execution, and nothing more. It is not a mod itself.

Computer mods build on it. Two consumers are planned: OpenComputers (OC), through a separate addon mod, and a new Retro Computing mod (RC), which will include Shuttle by default. Everything at the level of "a computer" (firmware, BIOS/OS boot, devices, ticks, the save format of a whole machine) belongs to those mods, not to Shuttle.

## How it works

- **Persistence:** Weave transforms a Wasm module so that a running program can be unwound into its linear memory and rebuilt later. A host mod can therefore save a running guest with the world and continue it after a reload.
- **Java binding (`shuttle-java`):** Wasmtime and Weave through JNI: load a module, run it for a bounded number of polls, define host functions, take and restore snapshots. Planned extensions (yielding imports, memory limits, nested instances) are described in [docs/design/runtime-extensions.md](docs/design/runtime-extensions.md).
- **Targets:** Windows, macOS and Linux, each on x86_64 and arm64.

## Repository layout

| Path | What |
| --- | --- |
| [`shuttle-java/`](shuttle-java) | Java bindings for Wasmtime plus the Weave (checkpointable Wasm) JNI layer. See [docs/java-binding.md](docs/java-binding.md). |
| [`shuttle-java-example/`](shuttle-java-example) | Runnable Weave example: run a guest a few "ticks" at a time, snapshot it, restore it and continue. |
| [`shuttle-wit-parser/`](shuttle-wit-parser), [`shuttle-wit-maven-plugin/`](shuttle-wit-maven-plugin) | Build-time WIT tooling: generate Java component-context interfaces from WIT files. See [docs/wit-codegen.md](docs/wit-codegen.md). |
| [`native/`](native) | A prototype of a whole computer (firmware ABI, BIOS → OS → child programs) on Wasmtime + Weave, in Rust. It belongs to the RC mod and will move there; it is kept here for now as a reference. See [`native/README.md`](native/README.md). |
| [`docs/`](docs) | Documentation. Start at [docs/README.md](docs/README.md). |

The Java modules form one Maven reactor: groupId `jp.akkiserver`, parent artifactId `shuttle`, one artifact per directory (`shuttle-java`, `shuttle-java-example`, `shuttle-wit-parser`, `shuttle-wit-maven-plugin`). The version is set in the root [`pom.xml`](pom.xml).

## Quick start

Java bindings (requires JDK 21, Maven and Rust; Maven picks the native build for your OS and CPU automatically):

```sh
mvn -pl shuttle-java,shuttle-java-example -am install
mvn -pl shuttle-java-example exec:java
```

The second command runs the Weave example. Platform profiles, CI and known problems are covered in [docs/building.md](docs/building.md).

The `native/` prototype (requires Rust, plus clang and wasm-ld for the C guests):

```powershell
.\native\scripts\build.ps1          # Windows: build guests + workspace, then verify
```
```sh
./native/scripts/build.sh           # Linux / WSL2
```

`verify` boots BIOS → OS → child, then repeats the run with capture → restore into a fresh machine at every tick boundary and checks that the logs are identical.

## Documentation

- [docs/building.md](docs/building.md): building the Maven modules and the native/ prototype, CI, requirements, known issues
- [docs/java-binding.md](docs/java-binding.md): the `shuttle-java` API (Wasmtime and Weave) and its architecture
- [docs/design/runtime-extensions.md](docs/design/runtime-extensions.md): planned runtime extensions (design)
- [docs/wit-codegen.md](docs/wit-codegen.md): generating Java component contexts from WIT
- [native/README.md](native/README.md): the computer prototype (moving to the RC mod), its results and its ABI coverage

## Credits and license

The Java Wasmtime bindings in this repository are a fork of [StefanRichterHuber/wasmtime-ng](https://github.com/StefanRichterHuber/wasmtime-ng) (MIT, © 2024 Stefan Richter-Huber). See [LICENSE](LICENSE). Upstream names these earlier Java bindings as prior work: [bluejekyll/wasmtime-java](https://github.com/bluejekyll/wasmtime-java) and [kawamuray/wasmtime-java](https://github.com/kawamuray/wasmtime-java).

> **Disclaimer:** This project is an independent community effort and is not affiliated with, maintained, or endorsed by the [Wasmtime project](https://github.com/bytecodealliance/wasmtime), the Bytecode Alliance, Mojang or Microsoft.

> **AI Disclaimer:** Parts of this project were vibe-coded or written with the help of an AI assistant (Claude); the remaining parts were written by hand.
