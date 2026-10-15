package io.github.stefanrichterhuber.wasmtimejavang;

import java.util.Arrays;

/** Outcome of {@link WeaveInstance#call} / {@link WeaveInstance#resume}. */
public final class WeaveResult {
    private final boolean unwound;
    private final Object[] values;

    WeaveResult(boolean unwound, Object[] values) {
        this.unwound = unwound;
        this.values = values;
    }

    /** {@code true} if the guest unwound (checkpoint/yield) instead of finishing. */
    public boolean isUnwound() {
        return unwound;
    }

    public boolean isDone() {
        return !unwound;
    }

    /** Results (Integer/Long/Float/Double); empty when unwound. */
    public Object[] values() {
        return values.clone();
    }

    /** First result as int; throws if absent. */
    public int intValue() {
        return ((Number) values[0]).intValue();
    }

    public long longValue() {
        return ((Number) values[0]).longValue();
    }

    @Override
    public String toString() {
        return unwound ? "WeaveResult[unwound]" : "WeaveResult" + Arrays.toString(values);
    }

    static WeaveResult decode(long[] enc) {
        if (enc[0] == 1) {
            return new WeaveResult(true, new Object[0]);
        }
        int n = (int) enc[1];
        Object[] vals = new Object[n];
        for (int i = 0; i < n; i++) {
            long kind = enc[2 + 2 * i];
            long bits = enc[3 + 2 * i];
            vals[i] = switch ((int) kind) {
                case 0 -> (int) bits;
                case 1 -> bits;
                case 2 -> WeaveCaller.asFloat(bits);
                case 3 -> WeaveCaller.asDouble(bits);
                default -> throw new IllegalStateException("unknown value kind " + kind);
            };
        }
        return new WeaveResult(false, vals);
    }
}
