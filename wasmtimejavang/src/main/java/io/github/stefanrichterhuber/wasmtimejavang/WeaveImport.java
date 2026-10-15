package io.github.stefanrichterhuber.wasmtimejavang;

import java.util.List;

/** One host import definition. Accessors are called from native code. */
final class WeaveImport {
    private final String module;
    private final String name;
    private final List<ValType> params;
    private final List<ValType> results;
    private final WeaveHostFunction function;

    WeaveImport(String module, String name, List<ValType> params, List<ValType> results, WeaveHostFunction fn) {
        this.module = module;
        this.name = name;
        this.params = List.copyOf(params);
        this.results = List.copyOf(results);
        // Always invalidate the caller handle, even if the function throws.
        this.function = (caller, args) -> {
            try {
                long[] r = fn.call(caller, args);
                return r == null ? new long[0] : r;
            } finally {
                caller.invalidate();
            }
        };
    }

    String module() {
        return module;
    }

    String name() {
        return name;
    }

    List<ValType> params() {
        return params;
    }

    List<ValType> results() {
        return results;
    }

    WeaveHostFunction function() {
        return function;
    }
}
