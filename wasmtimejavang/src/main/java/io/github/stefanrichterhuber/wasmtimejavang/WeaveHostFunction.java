package io.github.stefanrichterhuber.wasmtimejavang;

/**
 * A Java function imported into a woven guest. Arguments and results are raw
 * bits: i32/i64 as-is (i32 sign-extended), f32/f64 via {@link WeaveCaller#fromFloat}
 * / {@link WeaveCaller#fromDouble} and the matching {@code as*} helpers.
 */
@FunctionalInterface
public interface WeaveHostFunction {
    /** @return result bits, one per declared result (may be {@code null} for none) */
    long[] call(WeaveCaller caller, long[] args);
}
