# WIT code generation

Two build-time modules generate typed Java interfaces from [WIT](https://component-model.bytecodealliance.org/design/wit.html) files (WIT is the interface description language of the WebAssembly Component Model). You implement the generated interface instead of writing a `WasmComponentContext` by hand (see [java-binding.md](java-binding.md#component-model)).

| Module | Role |
| --- | --- |
| [`shuttle-wit-parser`](../shuttle-wit-parser) | JNI binding (`witparserbinding`) to the Rust [`wit-parser`](https://crates.io/crates/wit-parser) crate. Package `jp.akkiserver.shuttle.wit.parser`. |
| [`shuttle-wit-maven-plugin`](../shuttle-wit-maven-plugin) | Maven plugin with the goal `generate-wit-sources`. Package `jp.akkiserver.shuttle.wit.codegen`. |

Neither module is needed at runtime: generated code only depends on `shuttle-java`.

## `shuttle-wit-parser`

```java
import jp.akkiserver.shuttle.wit.parser.*;

// One .wit file, or every .wit file directly in a directory (with an optional deps/ folder)
List<WitInterface> ifaces = WitParser.parse(Path.of("src/main/wit/greet.wit"));

// The flattened, transitive imports of a world in a WIT package directory
List<WitInterface> world = WitParser.resolveWorld(Path.of("wit/wasi-cli/wit"), "command");
```

The result types are records:

- `WitInterface(name, functions, resources)`: `name` is the bare interface name, e.g. `my:custom/greet`.
- `WitFunction(name, params, result)`
- `WitParam(name, type)`
- `WitValueType(kind, resourceName)`, where `kind` is a `WitTypeKind`

Supported: records, variants, enums, flags, options, results, tuples, lists, resources and all scalar types. Unsupported constructs throw: the WIT 0.3 async surface, `stream`/`future`, `map`, and fixed-length lists.

The native library is built for the platform Maven runs on (Linux, Windows or macOS, x86_64 or arm64); see [building.md](building.md#host-platform-profiles).

## `shuttle-wit-maven-plugin`

### Goal `generate-wit-sources`

The goal is bound to the `generate-sources` phase by default. Parameters:

| Parameter | Default | Meaning |
| --- | --- | --- |
| `targetPackage` (property `witCodegen.targetPackage`) | required | Java package of the generated interfaces. |
| `witSourceDirectory` | `${project.basedir}/src/main/wit` | Every `.wit` file directly in this directory is parsed and generated on its own. A missing directory is not an error. |
| `worldSourceDirectory` | none | A WIT package directory (optionally with `deps/`) to resolve a world from. Must be set together with `worldName`. |
| `worldName` | none | Bare world name, e.g. `command`. One interface is generated per interface that the world's imports flatten to. Exports are not modeled. |
| `outputDirectory` | `${project.build.directory}/generated-sources/wit-components` | Output folder. It is added as a compile source root when anything was generated. |

Plugin declaration:

```xml
<plugin>
    <groupId>jp.akkiserver</groupId>
    <artifactId>shuttle-wit-maven-plugin</artifactId>
    <version>6.0.1-SNAPSHOT</version> <!-- the version in the root pom.xml -->
    <executions>
        <execution>
            <goals>
                <goal>generate-wit-sources</goal>
            </goals>
            <configuration>
                <witSourceDirectory>${project.basedir}/src/main/wit</witSourceDirectory>
                <targetPackage>com.example.generated</targetPackage>
            </configuration>
        </execution>
    </executions>
</plugin>
```

World mode instead of (or in addition to) single files:

```xml
<configuration>
    <worldSourceDirectory>${project.basedir}/src/main/wit/wasi-cli/wit</worldSourceDirectory>
    <worldName>command</worldName>
    <targetPackage>com.example.generated.wasicli</targetPackage>
</configuration>
```

The plugin's tests generate 27 interfaces from `wasi:cli`'s `command` world this way.

From the command line, the full coordinates always work:

```sh
mvn jp.akkiserver:shuttle-wit-maven-plugin:6.0.1-SNAPSHOT:generate-wit-sources -DwitCodegen.targetPackage=com.example.generated
```

The plugin prefix is `shuttle-wit` (from the artifactId). `mvn shuttle-wit:generate-wit-sources` only resolves when the plugin is declared in the project's POM or `jp.akkiserver` is listed under `<pluginGroups>` in `settings.xml`.

### What gets generated

Input, [`greet.wit`](../shuttle-wit-maven-plugin/src/test/resources/wit/greet.wit):

```wit
package my:custom@1.0.0;

interface greet {
    hello: func(name: string) -> string;
    add: func(a: u32, b: u32) -> u32;
}
```

Output: `com.example.generated.GreetContext`. The class name is the interface name in PascalCase + `Context`. It is an **interface** that extends `WasmComponentContext`. It has:

- `INTERFACE = "my:custom/greet"`, and `name()` returning `"greet"`
- `default` implementations of `getImportFunctions()`, `getImportResources()` and `getProvidedInterfaces()`. Import names are built as `INTERFACE + "@" + getVersion()`.
- one abstract typed method per WIT function, with a `WasmtimeComponentInstance instance` first parameter
- one abstract `void drop<Resource>(int rep)` per resource

Method names:

- Free functions are prefixed with the interface name, so `hello` becomes `greetHello` and `add` becomes `greetAdd`. This avoids clashes when one class implements several interfaces that declare a function with the same name (for example `now` in both `monotonic-clock` and `wall-clock`).
- Resource methods are prefixed with the resource name, so `[method]input-stream.read` becomes `inputStreamRead`.
- Parameter names that are Java keywords get a trailing `_`.

Implementation:

```java
public class MyGreetContext implements com.example.generated.GreetContext {
    @Override
    public String greetHello(WasmtimeComponentInstance instance, String name) {
        return "Hello, " + name + "!";
    }

    @Override
    public int greetAdd(WasmtimeComponentInstance instance, int a, int b) {
        return a + b;
    }

    // The generated interface does not track the WIT package version.
    // WasmComponentContext.getVersion() defaults to 0.2.6, so return the
    // package version here or the imports are registered as
    // "my:custom/greet@0.2.6" instead of "@1.0.0".
    @Override
    public SemanticVersion getVersion() {
        return new SemanticVersion(1, 0, 0);
    }
}
```

Link it like any other context: `linker.linkContext(new MyGreetContext())`.

### Type mapping

| WIT | Parameter / result type |
| --- | --- |
| `bool` | `boolean` |
| `s8` `u8` `s16` `u16` `s32` `u32` | `int` |
| `s64` `u64` | `long` |
| `f32` / `f64` | `float` / `double` |
| `char` | `char` |
| `string` | `String` |
| `list<u8>` | `byte[]` |
| `list<T>` | `List<Object>` |
| `option<T>` | `Optional<Object>` |
| `result` | `WitResult` |
| `tuple` | `Object[]` |
| `record` | `Map<String, Object>` |
| `variant` | `WitVariant` |
| `enum` | `WitEnum` |
| `flags` | `Set<String>` |
| resource | `WitResource` |

Nested types (record fields, variant payloads and so on) are not reflected in the signature. The values arrive as the runtime types listed in [java-binding.md](java-binding.md#component-values).

### Combining interfaces

Interfaces were chosen over abstract classes so that one class can implement several generated interfaces and share state between them. Java then requires you to override the `default` methods that conflict, and to combine them:

```java
public class MyBundledContext implements FooContext, BarContext {
    @Override
    public List<ComponentImportFunction> getImportFunctions() {
        List<ComponentImportFunction> result = new ArrayList<>();
        result.addAll(FooContext.super.getImportFunctions());
        result.addAll(BarContext.super.getImportFunctions());
        return result;
    }
    // Same pattern for getImportResources(), getProvidedInterfaces() and name().
    // withVersion()/getVersion() stay this class's job (a plain field), since
    // interfaces cannot hold state.
}
```
