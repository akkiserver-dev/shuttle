package jp.akkiserver.shuttle;

import java.util.Objects;

/**
 * A woven (checkpointable) wasm module: the raw module is transformed once and
 * compiled once per engine; instances are cheap.
 *
 * <pre>{@code
 * try (WasmtimeEngine engine = new WasmtimeEngine();
 *      WeaveModule module = WeaveModule.load(engine, wasmBytes);
 *      WeaveInstance vm = module.instance()
 *              .define("sys", "log", "i>", (caller, a) -> { System.out.println(a[0]); return null; })
 *              .build()) {
 *     WeaveResult r = vm.call("run", 1, 2);
 * }
 * }</pre>
 */
public final class WeaveModule implements AutoCloseable {
    private final WasmtimeEngine engine;
    private long handle;

    private WeaveModule(WasmtimeEngine engine, long handle) {
        this.engine = engine;
        this.handle = handle;
    }

    /**
     * Transforms a raw core wasm module (binary or WAT text; no components) for
     * checkpointing, using the default poll period and shadow-stack size.
     */
    public static WeaveModule load(WasmtimeEngine engine, byte[] wasm) {
        return load(engine, wasm, 0, 0);
    }

    /**
     * @param pollPeriod how many back-edges/calls between unwind polls ({@code <= 0}: default)
     * @param stackPages shadow stack size in wasm pages ({@code <= 0}: default)
     */
    public static WeaveModule load(WasmtimeEngine engine, byte[] wasm, int pollPeriod, int stackPages) {
        Objects.requireNonNull(engine);
        Objects.requireNonNull(wasm);
        return new WeaveModule(engine, WeaveNative.moduleCreate(WeaveNative.direct(wasm), pollPeriod, stackPages));
    }

    /** Hash identifying the transformed module; a snapshot only restores into the same hash. */
    public byte[] hash() {
        return WeaveNative.moduleHash(handle());
    }

    /** Exported entry points as {@code name -> "params>results"} (type chars as in {@code define}). */
    public java.util.Map<String, String> entries() {
        java.util.Map<String, String> out = new java.util.LinkedHashMap<>();
        String all = WeaveNative.moduleEntries(handle());
        if (!all.isEmpty()) {
            for (String line : all.split("\n")) {
                int t = line.indexOf('\t');
                out.put(line.substring(0, t), line.substring(t + 1));
            }
        }
        return out;
    }

    /** Starts building an instance of this module. */
    public WeaveInstance.Builder instance() {
        return new WeaveInstance.Builder(this);
    }

    WasmtimeEngine engine() {
        return engine;
    }

    long handle() {
        if (handle == 0) {
            throw new IllegalStateException("WeaveModule is closed");
        }
        return handle;
    }

    @Override
    public void close() {
        if (handle != 0) {
            WeaveNative.moduleClose(handle);
            handle = 0;
        }
    }
}
