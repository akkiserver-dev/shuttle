# Working style

## Ask before deciding
- Before deciding or changing anything (specs, design, API names,
  constants, file layout, dependencies), ask the user first.
  Do not go ahead on your own.
- Even for exploratory work (trying something out, running an
  experiment), say what you are about to do before doing it.
- When asking, lay out the options with the pros and cons of each,
  and give a recommendation with the reason.
- After the work, list what changed and why, so every change can be followed.

## Explain concretely
- Do not stop at a short summary. Explain what happens and how it
  works, step by step.
- Include concrete examples: code, numbers, inputs and outputs, commands.
- Define technical terms the first time they appear.

## Role
- The user owns the project and makes the decisions. Claude is an
  assistant that helps; it does not take over.

## Branches
- New features: `feature/<name>`
- Fixes: `fix/<name>`

## Language
- Talk to the user in Japanese.
- Files in the repository (code, comments, docs, commit messages) are
  written in English unless the user says otherwise.

# Project

Shuttle is a WebAssembly runtime library (Java binding + native Rust:
Wasmtime + Weave) for Minecraft computer mods. It is not a mod itself.
OpenComputers will use it through an addon; a new Retro Computing mod
will include it by default. Current status and open decisions:
`native/HANDOFF.md`. Overview and docs: `README.md`, `docs/`.

| Path | What |
| --- | --- |
| `native/` | Rust workspace: `shuttle-core`, `shuttle-runner`, C test guests |
| `shuttle-java/` | `jp.akkiserver.shuttle`: Wasmtime + Weave JNI binding |
| `shuttle-java-example/` | Weave example |
| `shuttle-wit-parser/`, `shuttle-wit-maven-plugin/` | WIT tooling |

## Build and test (Windows)
- Native core: `.\native\scripts\build.ps1` (needs LLVM for clang and
  wasm-ld). Do not redirect its output; under Windows PowerShell 5.1 that
  makes cargo's progress output abort the script.
- Maven: there is no `mvn` on PATH. Use IntelliJ's bundled Maven,
  `%LOCALAPPDATA%\Programs\IntelliJ IDEA\plugins\maven-plugin\lib\maven3\bin\mvn.cmd`,
  with `JAVA_HOME=%USERPROFILE%\.jdks\temurin-25.0.4` (the global
  `JAVA_HOME` points at `bin\` and is wrong). No `-P` is needed: the root
  pom activates the `host-windows-x86-64` profile automatically. Quote
  `-D...` arguments in PowerShell (`"-Dcargo.profile=release"`).
- Tests run on this machine only; CI (`.github/workflows/ci.yml`) only builds,
  on six platforms.
- JNI class names are also written as strings in Rust (`java_type = "..."`).
  After renaming Java classes or packages, grep the `.rs` files and run the
  tests: a wrong name compiles but fails at run time.
