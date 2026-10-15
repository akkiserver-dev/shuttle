# Java binding (`shuttle-java`)

`shuttle-java` is the Java side of Shuttle that a host mod calls. It has two parts:

1. **Wasmtime bindings** (inherited from the upstream wasmtime-ng project): engine, modules, stores, linkers, instances, linear memory, host functions, and the Component Model.
2. **The Weave layer**: load a core Wasm module in *woven* (checkpointable) form, run it in slices, snapshot it into a byte array, and restore it later, possibly in another JVM.

All classes are in the package `jp.akkiserver.shuttle` (WIT value wrappers are in `jp.akkiserver.shuttle.component`). The native code is one Rust library, `wasmtimebinding` (source: [`shuttle-java/src/main/rust/wasmtime-binding`](../shuttle-java/src/main/rust/wasmtime-binding)). It is packaged inside the jar and loaded automatically.

How to build the jar, including which native platforms end up inside it, is described in [building.md](building.md).

## Dependency

The project is not published to a Maven repository yet. Install it locally (see [building.md](building.md)) and depend on:

```xml
<dependency>
    <groupId>jp.akkiserver</groupId>
    <artifactId>shuttle-java</artifactId>
    <version>6.0.1-SNAPSHOT</version> <!-- the version in the root pom.xml -->
</dependency>
```

Runtime dependencies: `org.questdb:jar-jni` (loads the native library) and `log4j-api`. Native log output from Rust is forwarded to Log4j2. A Log4j2 implementation is not included; inside Minecraft the game's own Log4j2 is used.

## Wasmtime API

| Class | Role |
| --- | --- |
| `WasmtimeEngine` | Wasmtime engine. Shared and reused across modules. `precompile(...)` produces a precompiled (`.cwasm`) image. |
| `WasmtimeModule` | Compiled core module. Constructors take a WAT `String`, `byte[]`, `ByteBuffer` or `InputStream` (binary or text). `fromPrecompiled(...)` loads a `.cwasm` image. |
| `WasmtimeStore` | Wasmtime store. Holds a user context `Map<String, Object>` (`getContext()`), reachable from host functions through the instance. |
| `WasmtimeLinker` | Defines imports: `linkFunction(...)`, `linkContext(WasmContext)`, `linkMemory(...)`. (`importFunction`, `link` and `defineSharedMemory` are deprecated aliases.) |
| `WasmtimeInstance` | Instantiated module: `invoke(name, args...)`, `start()` (calls `_start`), `getFunction(name)`, `getMemory(name)`. |
| `WasmtimeMemory` | Linear memory view over a `java.nio.ByteBuffer` (implemented by `WasmtimeLocalMemory` and `WasmtimeSharedMemory`). |
| `WasmtimeFunction` | Functional interface `Object[] call(WasmtimeInstance instance, Object... args)` used for host functions and for exported function references. |
| `WasmContext` | A reusable bundle of import functions (`ImportFunction`) and shared memories (`Importmemory`) for `linkContext`. |

All native-backed classes implement `AutoCloseable`. Use try-with-resources.

### Host functions

A host function is a Java method that a Wasm module imports:

```java
import java.util.List;
import jp.akkiserver.shuttle.*;

String wat = """
    (module
      (func $hello (import "env" "hello"))
      (func (export "run") (call $hello))
    )
    """;

try (
    WasmtimeEngine engine = new WasmtimeEngine();
    WasmtimeModule module = new WasmtimeModule(engine, wat);
    WasmtimeStore store = new WasmtimeStore(engine);
    WasmtimeLinker linker = new WasmtimeLinker(engine, store)
) {
    store.getContext().put("greeting", "Hello world");

    // env.hello: no parameters, no results
    linker.linkFunction("env", "hello", List.of(), List.of(), (instance, params) -> {
        System.out.println("Java side: " + instance.getContext().get("greeting"));
        return new Object[] {};
    });

    try (WasmtimeInstance instance = new WasmtimeInstance(store, module, linker)) {
        instance.invoke("run"); // prints "Java side: Hello world"
    }
}
```

Parameter and result types are given as `ValType` values: `I32`, `I64`, `F32`, `F64`, `V128`, `Ref` (externref).

### Supported value types

Conversions work in both directions unless noted otherwise.

| Wasm type | Java type | Notes |
| :--- | :--- | :--- |
| `i32` | `java.lang.Integer` | Numbers are always boxed. |
| `i64` | `java.lang.Long` | |
| `f32` | `java.lang.Float` | |
| `f64` | `java.lang.Double` | |
| `v128` | `jp.akkiserver.shuttle.V128` | Wrapper around `byte[16]`, with conversions from and to `byte[]`, `short[]`, `int[]`, `long[]` and `BigInteger`. |
| `funcref` | `jp.akkiserver.shuttle.WasmtimeFunction` | Callable function reference. Only Wasm → Java (exported function references) is supported. |
| `externref` | any Java object | Any Java object can be passed through Wasm as an externref (`ValType.Ref`). |

### Memory

`WasmtimeMemory` maps linear memory directly to a `java.nio.ByteBuffer`:

```java
try (WasmtimeInstance instance = new WasmtimeInstance(store, module, linker)) {
    WasmtimeMemory mem = instance.getMemory("memory");

    byte[] data = mem.read(0x1000, 4);          // 4 bytes from offset 0x1000
    mem.write(0x1003, (byte) 5);                // one byte at 0x1003

    // Normally the guest's own allocator would hand out these addresses.
    mem.writeCString(0x1000, "Hello", StandardCharsets.UTF_8);    // NUL-terminated
    mem.readCString(0x1000, StandardCharsets.UTF_8);              // "Hello"

    mem.writeString(0x1000, "World", StandardCharsets.UTF_8);     // no terminator
    mem.readString(0x1000, 5, StandardCharsets.UTF_8);            // "World"

    mem.writeLong(0x1000, Long.MAX_VALUE);   mem.readLong(0x1000);
    mem.writeInt(0x1000, Integer.MAX_VALUE); mem.readInt(0x1000);
    mem.writeShort(0x1000, Short.MAX_VALUE); mem.readShort(0x1000);
    mem.write(0x1000, Byte.MAX_VALUE);       mem.readByte(0x1000);

    mem.grow(1);                             // add one 64 KiB page
}
```

`createView(address, length)` returns a `WasmtimeMemory` over a sub-range. `WasmtimeSharedMemory(engine, initialPages, maxPages)` creates a shared memory that can be imported with `linker.linkMemory(module, name, memory)`.

### WASI

`shuttle-java` currently contains **no WASI implementation**. Upstream implemented WASI Preview 1 (`WasiPI1Context`, `WasiThreadContext`) and WASI Preview 2 (`wasmtimejavang-wasi-*` modules) in Java. All of these were removed in commit `44c4363` ("remove wasi components"). Shuttle guests talk to the host through the firmware ABI instead of WASI. A module that imports WASI functions only links if the host provides those imports itself, for example with `linkFunction`.

## Component Model

The Component Model plumbing is still present, without any built-in interfaces:

| Class | Role |
| --- | --- |
| `WasmtimeComponent` | Compiled component (WAT or binary from `String` bytes, `byte[]`, `ByteBuffer`, `InputStream`). `getImportInterfaces()` / `getExportInterfaces()` read the named interfaces straight from the compiled component, without instantiating it. |
| `WasmtimeComponentLinker` | `linkContext(WasmComponentContext)` links one context (after its declared dependencies). `linkRequired(component)` links a provider for every import interface not already covered, found through a `ComponentContextLookup`. It throws `IllegalStateException` if an interface stays unresolved. |
| `WasmtimeComponentInstance` | Instantiated component. `invoke(interfaceName, funcName, args...)` calls an export; use `""` as the interface name for root-level exports. |
| `WasmComponentContext` | A host-side implementation of one or more component interfaces. |

### Component values

| WIT type | Java type |
| :--- | :--- |
| `bool` | `Boolean` |
| `s8`, `u8`, `s16`, `u16`, `s32`, `u32` | `Integer` |
| `s64`, `u64` | `Long` |
| `f32` / `f64` | `Float` / `Double` |
| `char` | `Character` |
| `string` | `String` |
| `list<u8>` | `byte[]` |
| `list<T>` | `List<Object>` |
| `tuple<...>` | `Object[]` |
| `record` | `Map<String, Object>` (field name → value) |
| `variant` | `jp.akkiserver.shuttle.component.WitVariant(caseName, value)` |
| `enum` | `jp.akkiserver.shuttle.component.WitEnum(name)` |
| `option<T>` | `Optional<Object>` |
| `result<T, E>` | `jp.akkiserver.shuttle.component.WitResult(ok, value)` |
| `flags` | `Set<String>` |
| resource handle | `jp.akkiserver.shuttle.component.WitResource(resourceName, rep, owned)` |

Unsigned values use the signed Java type of the same width (a `u32` above `Integer.MAX_VALUE` arrives as a negative `Integer`).

### Writing a `WasmComponentContext`

`WasmComponentContext` methods:

| Method | Default | Purpose |
| --- | --- | --- |
| `getImportFunctions()` | (abstract) | `ComponentImportFunction(interfaceName, funcName, ComponentFunction)` entries. `interfaceName` is the versioned name, e.g. `my:custom/greet@1.0.0`, or `""` for root imports. |
| `getImportResources()` | (abstract) | `ComponentImportResource(interfaceName, resourceName, ResourceDestructor)` entries. The destructor gets the resource `rep` when the guest drops an owned handle. |
| `name()` | (abstract) | Stable id. Used as the linker's cache key and as the name other contexts depend on. |
| `getDependencies()` | empty | Names of other contexts that must be linked first. |
| `onDependenciesResolved(lookup)` | no-op | Called after the dependencies are linked; look them up by name here. |
| `getProvidedInterfaces()` | empty | Bare (unversioned) interface names this context implements. `linkRequired` uses it to see what is already covered and to auto-discover providers. |
| `withVersion` / `getVersion` | no-op / `0.2.6` | The version used to build versioned interface names. |
| `getMiniumVersion` / `getMaximumVersion` | `0.0.0` / max | Range accepted by `supportsVersion(...)`. (The misspelling `Minium` is the actual method name.) |

Example, taken from `WasmtimeComponentTest`: a component imports `is-even(u32) -> bool` at the root and re-exports it as `run`. The Java side implements `is-even`.

```java
try (
    WasmtimeEngine engine = new WasmtimeEngine();
    WasmtimeComponent component = new WasmtimeComponent(engine, COMPONENT_WAT.getBytes(StandardCharsets.UTF_8));
    WasmtimeStore store = new WasmtimeStore(engine);
    WasmtimeComponentLinker linker = new WasmtimeComponentLinker(engine, store)
) {
    linker.linkContext(new WasmComponentContext() {
        @Override
        public List<ComponentImportFunction> getImportFunctions() {
            return List.of(new ComponentImportFunction("", "is-even", (instance, args) -> {
                int x = (Integer) args[0];
                return new Object[] { x % 2 == 0 };
            }));
        }

        @Override
        public List<ComponentImportResource> getImportResources() { return List.of(); }

        @Override
        public String name() { return "test-context"; }
    });

    try (WasmtimeComponentInstance instance = new WasmtimeComponentInstance(store, component, linker)) {
        instance.invoke("", "run", 4);  // -> [true]
        instance.invoke("", "run", 5);  // -> [false]
    }
}
```

To avoid writing `getImportFunctions()` and the casts by hand, generate a typed interface from a WIT file with `shuttle-wit-maven-plugin`. See [wit-codegen.md](wit-codegen.md).

### Discovery and overriding

- **Auto-discovery:** `linkRequired(component)` asks the configured `ComponentContextLookup`. The default, `ServiceLoaderComponentContextLookup`, loads every class listed in `META-INF/services/jp.akkiserver.shuttle.WasmComponentContext` and picks the first whose `getProvidedInterfaces()` contains the bare interface name and whose `supportsVersion(...)` accepts the version requested by the component. A provider class needs a public no-arg constructor or a public static `provider()` method.
- **Explicit wins:** `linkRequired` skips every interface already covered by a context linked with `linkContext(...)`. That means an explicitly linked context with the same interfaces always takes precedence. It also means an explicitly linked context must declare `getProvidedInterfaces()`, otherwise `linkRequired` cannot see that the interface is covered and throws.
- **Global override:** register your own `ComponentContextLookup` in `META-INF/services/jp.akkiserver.shuttle.ComponentContextLookup`. The linker uses it instead of `ServiceLoaderComponentContextLookup`.

## Weave: checkpointable guests

The Weave layer lets a host run a guest a little at a time, stop it at any *poll point* (Weave inserts polls at loop back-edges and calls), save the complete guest state, and continue later from exactly that point. This is what lets a computer survive a world save and reload.

| Class | Role |
| --- | --- |
| `WeaveModule` | `WeaveModule.load(engine, wasm)` or `load(engine, wasm, pollPeriod, stackPages)`. Takes a core module, binary or WAT (no components), and transforms and compiles it once. `pollPeriod` is the number of back-edges/calls between polls and `stackPages` the shadow-stack size in Wasm pages (`<= 0` means the Weave default for either). `hash()` identifies the transformed module; `entries()` lists the exports as `name -> "params>results"`; `instance()` returns a builder. |
| `WeaveInstance.Builder` | `define(module, name, signature, WeaveHostFunction)` adds a host import. `build()` creates a fresh instance (runs the start function); `restore(snapshot)` creates one from a snapshot. |
| `WeaveInstance` | A running guest (not thread-safe). `call(entry, args...)`, `resume()`, `unwindNext()`, `unwindAfter(polls)`, `snapshot()`, `state()`, `readMemory`/`writeMemory`, `cancellation()`. |
| `WeaveInstance.State` | `READY`, `RESTORE_TARGET`, `RESTORING`, `RUNNING`, `PAUSED`, `COMPLETED`, `RETIRED`, `FAILED`. |
| `WeaveInstance.Cancellation` | Thread-safe handle: `cancel()` makes the guest unwind at its next poll. Close it before closing the instance. |
| `WeaveHostFunction` | `long[] call(WeaveCaller caller, long[] args)`. Arguments and results are raw bits. Return one value per declared result, or `null` when there are none. |
| `WeaveCaller` | Valid only during a host call: `read`, `write`, `readString` (UTF-8) on the guest's primary memory, and `requestUnwind()` to suspend the guest at its next poll. Static helpers `asFloat`/`asDouble`/`fromFloat`/`fromDouble` convert between floats and raw bits. |
| `WeaveResult` | Result of `call`/`resume`: `isUnwound()` / `isDone()`, `values()` (`Integer`/`Long`/`Float`/`Double`), `intValue()`, `longValue()`. |

**Signatures** are written `params>results` with one character per value: `i` = i32, `l` = i64, `f` = f32, `d` = f64. Examples: `"ii>i"` takes two i32 and returns one i32; `"i>"` takes one i32 and returns nothing. `entries()` may also show `v` (v128) and `r` (funcref), which `define` and `call` do not support. `call` accepts `Integer`, `Long`, `Float` and `Double` arguments only.

**Raw bits:** an i32 arrives sign-extended in a `long`, so `(int) a[0]` recovers it. An f32 is passed as its IEEE bit pattern; use `WeaveCaller.asFloat(a[0])`, and `WeaveCaller.fromFloat(x)` for results.

### Example: run in tick slices, save, restore

Condensed from [`VirtualComputerExample`](../shuttle-java-example/src/main/java/example/VirtualComputerExample.java). Its guest [`guest.wat`](../shuttle-java-example/src/main/resources/guest.wat) exports `main(n)`, which loops `n` times, calls `sys.print` and adds the loop counter through `sys.add`.

```java
static WeaveInstance.Builder builder(WeaveModule module) {
    return module.instance()
        .define("sys", "print", "ii>", (caller, a) -> {           // (ptr, len)
            System.out.println(caller.readString(a[0], (int) a[1]));
            return null;
        })
        .define("sys", "add", "ii>i", (caller, a) -> new long[] { (int) a[0] + (int) a[1] });
}

try (WasmtimeEngine engine = new WasmtimeEngine();
     WeaveModule module = WeaveModule.load(engine, wasm, 1, 0)) {   // poll on every back-edge

    byte[] snapshot;
    try (WeaveInstance vm = builder(module).build()) {
        vm.unwindAfter(2);                     // budget for this tick: 2 polls
        WeaveResult r = vm.call("main", 12);
        if (r.isUnwound()) {                   // stopped mid-loop
            snapshot = vm.snapshot();          // gzip-compressed byte[]
        } else {
            return;                            // finished within one tick
        }
    }

    // Later, possibly in another JVM: same module, same host imports.
    try (WeaveInstance vm = builder(module).restore(snapshot)) {
        WeaveResult r;
        do {
            vm.unwindAfter(2);                 // one tick
            r = vm.resume();
        } while (r.isUnwound());
        System.out.println(r.intValue());      // 66 = 0 + 1 + ... + 11
    }
}
```

Run the full example with `mvn -pl shuttle-java-example exec:java`. Its comments are in Japanese.

Rules that follow from the API:

- `snapshot()` only works while the guest is paused (unwound). The snapshot is gzip-compressed and small enough to store as a byte array, for example in an NBT `ByteArrayTag`.
- A snapshot only restores into a module with the same `hash()`, and `restore` must be given the same host function definitions as the original instance.
- State kept on the Java side, such as fields captured by host function lambdas, is **not** part of the snapshot.
- Ways to stop a guest: `unwindNext()` / `unwindAfter(n)` before `call`/`resume` (a budget), `caller.requestUnwind()` from inside a host function (a yield), or `cancellation().cancel()` from another thread (a power-off).

### Relation to `native/`

The Weave layer in `shuttle-java` and the native core in `native/` are separate. `shuttle-java` pins Weave commit `0f26893`, which does not have the `shuttle-host-hooks` additions (yielding imports, memory limits). `native/` pins `677cce6` on branch `shuttle-host-hooks`. How the Java layer will connect to `shuttle-core` (a Machine API over JNI) has not been designed yet.

## Architecture

`shuttle-java` has three layers:

1. **Java API**: type-safe wrappers for the Wasmtime and Weave concepts above. `jar-jni` loads the platform library from `jp/akkiserver/shuttle/libs/<platform>/` inside the jar. Log4j2 is the logging API. Native handles are released by `close()`, with a `Cleaner` as a fallback for the Wasmtime classes.
2. **JNI layer (Rust)**: built with the [jni](https://crates.io/crates/jni) crate (`bind_java_type!`). It owns the native Wasmtime objects, converts values between the JVM and Rust (for example `wasmcomponentvalue.rs` for component values), and forwards Rust `log` output to Log4j2. Classic JNI was chosen over the Foreign Function and Memory API because the library is only used from Java: Rust can call Java methods and construct Java objects directly, and keeps type and lifetime checks on the native side. [`weave.rs`](../shuttle-java/src/main/rust/wasmtime-binding/src/weave.rs) is the Weave part. Weave uses its own store and linker; only the `Engine` is shared with `WasmtimeEngine`. All values cross the boundary as raw `long` bits, so no boxing is needed. A host function call attaches the current thread to the JVM and calls `WeaveHostFunction.call`.
3. **Wasmtime**: the [wasmtime](https://crates.io/crates/wasmtime) crate (version 46), plus the `weave-*` crates from [akkiserver-dev/weave](https://github.com/akkiserver-dev/weave).
