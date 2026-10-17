# Shuttle documentation

| Document | Contents |
| --- | --- |
| [building.md](building.md) | Requirements; building the Maven modules and their platform profiles; building the `native/` prototype (`native/scripts/build.ps1`, `build.sh`, what `verify` checks); CI; known issues. |
| [java-binding.md](java-binding.md) | The `shuttle-java` module: Wasmtime API (engine, module, store, linker, instance, memory, functions), value types, Component Model support, the Weave JNI layer for checkpointable guests, and the architecture (Java API / JNI layer in Rust / Wasmtime). |
| [wit-codegen.md](wit-codegen.md) | `shuttle-wit-parser` and `shuttle-wit-maven-plugin`: generating Java component-context interfaces from WIT files. |
| [design/runtime-extensions.md](design/runtime-extensions.md) | Design (not implemented): yielding imports, memory limits and nested-instance primitives for the Weave binding. |
| [../native/README.md](../native/README.md) | The computer prototype (`shuttle-core`, `shuttle-runner`, C guests), which will move to the Retro Computing mod: layout, build, test results, firmware ABI coverage. |

Back to the [project README](../README.md).
