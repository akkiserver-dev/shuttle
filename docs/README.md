# Shuttle documentation

| Document | Contents |
| --- | --- |
| [building.md](building.md) | Requirements; building the native core (`native/scripts/build.ps1`, `build.sh`, what `verify` checks); building the Maven modules and their profiles; CI workflows; known issues. |
| [java-binding.md](java-binding.md) | The `shuttle-java` module: Wasmtime API (engine, module, store, linker, instance, memory, functions), value types, Component Model support, the Weave JNI layer for checkpointable guests, and the architecture (Java API / JNI layer in Rust / Wasmtime). |
| [wit-codegen.md](wit-codegen.md) | `shuttle-wit-parser` and `shuttle-wit-maven-plugin`: generating Java component-context interfaces from WIT files. |
| [../native/README.md](../native/README.md) | The native core prototype (`shuttle-core`, `shuttle-runner`, C guests): layout, build, test results, and which parts of the firmware ABI draft are implemented. |

Back to the [project README](../README.md).
