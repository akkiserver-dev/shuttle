# Building

The repository has two independent builds:

- **`native/`**: a Cargo workspace (the Shuttle native core). Built with the scripts in `native/scripts/`.
- **The Maven reactor** (root `pom.xml`): `shuttle-java`, `shuttle-java-example`, `shuttle-wit-parser`, `shuttle-wit-maven-plugin`. Maven also drives the Rust builds of the two JNI libraries inside `shuttle-java` and `shuttle-wit-parser`.

Neither build depends on the other.

## Requirements

| Tool | Needed for |
| --- | --- |
| Rust via [rustup](https://rustup.rs) (stable) | everything |
| clang + wasm-ld | the C guests in `native/guests`. Windows: the [LLVM installer](https://github.com/llvm/llvm-project/releases). Linux/WSL2: `sudo apt install clang lld` |
| JDK 21 and Maven | the Maven reactor (`maven.compiler.release` is 21) |
| Rust target `wasm32-unknown-unknown` | the `shuttle-java` tests (the `valuetest` fixture is built for it): `rustup target add wasm32-unknown-unknown` |
| [cross](https://github.com/cross-rs/cross) + Docker | only `native/scripts/build.sh --cross` (the Maven build does not use it) |

## Native core (`native/`)

The scripts build the three C guests (`bios`, `os`, `child`) into freestanding wasm32 modules under `native/guests/out/`, run `cargo build --locked`, and then run `shuttle-runner verify`.

Windows (PowerShell):

```powershell
.\native\scripts\build.ps1               # dev build + verify
.\native\scripts\build.ps1 -Release      # release build + verify
.\native\scripts\build.ps1 -SkipVerify   # build only
```

If `clang` is not on `PATH`, `build.ps1` adds `%ProgramFiles%\LLVM\bin`, because the LLVM installer does not always do this.

Linux / WSL2:

```sh
./native/scripts/build.sh                     # dev build + verify
./native/scripts/build.sh --release           # release build + verify
./native/scripts/build.sh --cross aarch64     # Linux arm64 via cross (build only, no verify)
./native/scripts/build.sh --skip-verify       # build only
```

By hand, inside `native/`:

```sh
sh guests/build.sh
cargo build
./target/debug/shuttle-runner run    guests/out/bios.wasm guests/out/os.wasm guests/out/child.wasm
./target/debug/shuttle-runner verify guests/out/bios.wasm guests/out/os.wasm guests/out/child.wasm
```

### What `verify` checks

`run` boots BIOS → `exec` → OS → `co_create` → child and prints the log until power-off.

`verify` does the following:

1. Runs the machine once without interruption and records the log.
2. For every tick boundary *N*, runs again, captures a snapshot at tick *N*, restores it into a brand-new machine, continues to power-off, and compares the log with step 1. All must be identical.
3. A control run drops the host-side child table on restore. That run must diverge, which shows that the comparison actually detects lost state.

The results of the last measured run are in [native/README.md](../native/README.md#results-2026-10-15-linux-x86_64-2-cores-dev-profile).

## Maven reactor

### Host platform profiles

The two JNI libraries, `wasmtimebinding` (in `shuttle-java`) and `witparserbinding` (in `shuttle-wit-parser`), are built with plain `cargo` during the `compile` phase, for the platform Maven runs on. They are copied into the jar under `.../libs/<platform>/`, where [jar-jni](https://github.com/questdb/jar-jni) finds them at runtime.

The root [`pom.xml`](../pom.xml) has one profile per platform. Maven activates the right one automatically from the JVM's `os.name` / `os.arch`, so no `-P` is needed:

| Profile | Rust target | Jar folder | Library |
| --- | --- | --- | --- |
| `host-linux-x86-64` | `x86_64-unknown-linux-gnu` | `linux-x86-64` | `.so` |
| `host-linux-aarch64` | `aarch64-unknown-linux-gnu` | `linux-aarch64` | `.so` |
| `host-windows-x86-64` | `x86_64-pc-windows-msvc` | `win32-x86-64` | `.dll` |
| `host-windows-aarch64` | `aarch64-pc-windows-msvc` | `win32-aarch64` | `.dll` |
| `host-macos-x86-64` | `x86_64-apple-darwin` | `darwin-x86-64` | `.dylib` |
| `host-macos-aarch64` | `aarch64-apple-darwin` | `darwin-aarch64` | `.dylib` |

Each profile only sets three properties (`native.rust.target`, `native.libdir`, `native.lib.ext`); the cargo and copy steps are written once in each module's `pom.xml`. Check which profile is active with:

```sh
mvn -N help:active-profiles
```

A jar built this way contains the library for one platform only. Collecting the libraries of all six platforms into one jar is not set up yet.

Other profiles in the root `pom.xml`:

| Profile | Effect |
| --- | --- |
| `release` (also activated by `-DperformRelease=true`) | Sets the Cargo profile to `release`, and adds GPG signing, source and javadoc jars, and the release plugin. GPG signing runs in the `verify` phase, so `install` with this profile needs a configured GPG key. |
| `spotbugs` | Runs SpotBugs with FindSecBugs in the `test` phase (`spotbugs.failOnError` is `false`). |

To get optimized Rust libraries without the release-publishing steps, set the two Cargo properties directly, as CI does:

```sh
mvn -Dcargo.profile=release -Dcargo.profile.folder=release install
```

### Commands

The same on every platform:

```sh
mvn test                                      # build and run all tests
mvn install                                   # build, test and install into ~/.m2
mvn -pl shuttle-java,shuttle-java-example -am install   # only shuttle-java and the example
mvn -pl shuttle-java-example exec:java        # run the Weave example (after install)
```

On Windows, run these from PowerShell with each `-D...` argument quoted (`"-Dcargo.profile=release"`), because PowerShell splits unquoted arguments at the dot.

### Tests

Tests are run on the developer machine; CI only builds.

- `shuttle-java`: JUnit 5 tests (`WeaveTest`, `WasmtimeValueTest`, `WasmtimeComponentTest`, `WasmtimeMemoryTest`, ...). In `generate-test-resources`, `exec-maven-plugin` builds the Rust fixture `src/test/rust/valuetest` for `wasm32-unknown-unknown` into `target/rust-test/valuetest`. The other fixtures are inline WAT or inline Wasm bytes.
- `shuttle-wit-parser` and `shuttle-wit-maven-plugin`: parse the WIT files under `src/test/resources/wit` (`greet.wit`, `wasi-cli`, `wasi-sockets`) and compile the generated sources.
- Coverage is recorded with JaCoCo (`prepare-agent`, report in `prepare-package`).

## CI

One workflow, [`ci.yml`](../.github/workflows/ci.yml) "CI", runs on pushes to `main`, on every pull request, and on manual dispatch. It only builds; it runs no tests. `setup-rust-toolchain` sets `RUSTFLAGS=-D warnings`, so Rust warnings fail the build.

| Job | Runs on | What it does |
| --- | --- | --- |
| `guests` | Ubuntu | Builds the C guests (`native/guests/build.sh`). |
| `native (<platform>)` | the six platforms below | `cargo build --locked --release` in `native/`. |
| `java (<platform>)` | the six platforms below | JDK 21 (Temurin): `mvn -B -DskipTests -Dcargo.profile=release -Dcargo.profile.folder=release package`, which builds the JNI libraries for that platform. |

Platforms (GitHub-hosted runners): `ubuntu-latest`, `ubuntu-24.04-arm`, `windows-latest`, `windows-11-arm`, `macos-15-intel`, `macos-latest`.

## Known issues

- The JNI crates allow the `deprecated` lint: jni 0.22.4's `bind_java_type!` expands to `AtomicBool::fetch_update`, deprecated in Rust 1.99 ([jni-rs/jni-rs#846](https://github.com/jni-rs/jni-rs/issues/846)). Remove the `#![allow(deprecated)]` lines once jni ships a fix.
