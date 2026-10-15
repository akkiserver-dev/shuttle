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
| [cross](https://github.com/cross-rs/cross) + Docker | only the Maven profiles that cross-compile (see below), and `build.sh --cross` |

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

### Profiles

The native library of `shuttle-java` (`wasmtime-binding`, built as `wasmtimebinding`) is compiled during the `compile` phase by a profile, and copied into the jar under `jp/akkiserver/shuttle/libs/<platform>/`, where [jar-jni](https://github.com/questdb/jar-jni) finds it at runtime. Profiles defined in [`shuttle-java/pom.xml`](../shuttle-java/pom.xml):

| Profile | Rust target | Built with | Jar folder |
| --- | --- | --- | --- |
| `build-linux-x86-64` (**active by default**) | `x86_64-unknown-linux-gnu` | `cross` (Docker) | `linux-x86-64` |
| `build-linux-aarch64` | `aarch64-unknown-linux-gnu` | `cross` (Docker) | `linux-aarch64` |
| `build-windows-x86_64` | `x86_64-pc-windows-msvc` | `cross` (Docker) | `win32-x86-64` |
| `build-windows-aarch64` | `aarch64-pc-windows-msvc` | `cross` (Docker) | `win32-aarch64` |
| `build-host-windows-x86_64` | `x86_64-pc-windows-msvc` | local `cargo` (no Docker) | `win32-x86-64` |

There is no macOS profile yet, so `shuttle-java` cannot currently be packaged with a macOS library.

Maven turns off an `activeByDefault` profile as soon as another profile of the same POM is selected with `-P`. So `-P build-host-windows-x86_64` builds only the Windows library and does not call `cross`.

Profiles defined in the root [`pom.xml`](../pom.xml):

| Profile | Effect |
| --- | --- |
| `release` (also activated by `-DperformRelease=true`) | Sets the Cargo profile to `release`, and adds GPG signing, source and javadoc jars, and the release plugin. GPG signing runs in the `verify` phase, so `install` with this profile needs a configured GPG key. |
| `spotbugs` | Runs SpotBugs with FindSecBugs in the `test` phase (`spotbugs.failOnError` is `false`). |

To get optimized Rust libraries without the release-publishing steps, set the two Cargo properties directly, as CI does:

```sh
mvn -Dcargo.profile=release -Dcargo.profile.folder=release install
```

### Commands

Windows, local cargo only:

```powershell
# shuttle-java and the example (with shuttle-java's tests)
mvn -P build-host-windows-x86_64 -pl shuttle-java,shuttle-java-example -am install

# the whole reactor, skipping tests (see Known issues for why)
mvn -P build-host-windows-x86_64 -DskipTests install
```

Linux, default profile (needs `cross` and a running Docker):

```sh
mvn install
```

Several platforms in one jar:

```sh
mvn -P build-linux-x86-64,build-linux-aarch64,build-windows-x86_64 install
```

Run the Weave example (after `shuttle-java` is installed in the local repository):

```sh
mvn -pl shuttle-java-example exec:java
```

### `shuttle-wit-parser`

`shuttle-wit-parser` always builds its JNI library (`witparserbinding`) with a plain host `cargo build`, without `cross` or a profile, because it only runs on the build machine while the WIT plugin is generating sources.

### Tests

- `shuttle-java`: JUnit 5 tests (`WeaveTest`, `WasmtimeValueTest`, `WasmtimeComponentTest`, `WasmtimeMemoryTest`, ...). In `generate-test-resources`, `exec-maven-plugin` builds the Rust fixture `src/test/rust/valuetest` for `wasm32-unknown-unknown` into `target/rust-test/valuetest`. The other fixtures are inline WAT or inline Wasm bytes.
- `shuttle-wit-parser` and `shuttle-wit-maven-plugin`: parse the WIT files under `src/test/resources/wit` (`greet.wit`, `wasi-cli`, `wasi-sockets`) and compile the generated sources.
- Coverage is recorded with JaCoCo (`prepare-agent`, report in `prepare-package`).

## CI

| Workflow | Trigger | What it does |
| --- | --- | --- |
| [`native.yml`](../.github/workflows/native.yml) "Native (Shuttle)" | push to `main` or PR touching `native/**` or the workflow; manual dispatch | Builds the C guests once on Ubuntu and uploads them as an artifact. Then, on Linux, Windows and macOS (x86_64 and arm64 each, six jobs): `cargo build --locked --release` and `shuttle-runner verify`. |
| [`maven.yml`](../.github/workflows/maven.yml) "Java CI with Maven" | push or PR to `main` | Ubuntu, JDK 21 (Temurin): installs `cross`, then runs `mvn -P build-linux-x86-64,build-linux-aarch64,build-windows-x86_64 -Dcargo.profile=release -Dcargo.profile.folder=release -B package`. |

## Known issues

- **`shuttle-wit-parser` tests fail on Windows.** Its `pom.xml` only copies the Linux x86_64 library (`*.so` into `jp/akkiserver/shuttle/wit/parser/libs/linux-x86-64`). On Windows the DLL is built but never copied into the jar folder, so loading fails with `cannot find .../win32-x86-64/witparserbinding.dll`. The `shuttle-wit-maven-plugin` tests use the parser too. On Windows, skip tests for these modules (`-DskipTests`) or build only `shuttle-java` (`-pl shuttle-java -am`).
- **"Java CI with Maven" is failing**, also on `main`. The cause has not been investigated yet. Possible factors: the workflow installs `wasm32-wasip1`/`wasm32-wasip2` targets (left over from the removed WASI modules) but not `wasm32-unknown-unknown`, which the `valuetest` fixture needs, and it cross-compiles `x86_64-pc-windows-msvc` with `cross`.
- **No macOS Maven profile.** `native/` builds on macOS in CI, but `shuttle-java` has no profile that builds or packages a macOS library.
