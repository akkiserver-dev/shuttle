package jp.akkiserver.shuttle;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.UncheckedIOException;
import java.util.ArrayList;
import java.util.List;
import java.util.zip.GZIPInputStream;
import java.util.zip.GZIPOutputStream;

/**
 * A running, checkpointable guest. Not thread-safe; use {@link #cancellation()}
 * to interrupt from another thread.
 *
 * <pre>{@code
 * vm.unwindNext();                       // checkpoint at the next poll
 * WeaveResult r = vm.call("run");
 * if (r.isUnwound()) {
 *     byte[] snap = vm.snapshot();       // gzip-compressed, fits an NBT ByteArrayTag
 *     // ... later, possibly in another JVM:
 *     try (WeaveInstance vm2 = module.instance().restore(snap)) { vm2.resume(); }
 * }
 * }</pre>
 */
public final class WeaveInstance implements AutoCloseable {
    /** Lifecycle state of the guest. */
    public enum State { READY, RESTORE_TARGET, RESTORING, RUNNING, PAUSED, COMPLETED, RETIRED, FAILED }

    private long handle;

    private WeaveInstance(long handle) {
        this.handle = handle;
    }

    private long h() {
        if (handle == 0) {
            throw new IllegalStateException("WeaveInstance is closed");
        }
        return handle;
    }

    /** Calls an exported entry. Args: Integer, Long, Float or Double. */
    public WeaveResult call(String entry, Object... args) {
        long[] enc = new long[args.length * 2];
        for (int i = 0; i < args.length; i++) {
            Object a = args[i];
            if (a instanceof Integer v) {
                enc[2 * i] = 0;
                enc[2 * i + 1] = v;
            } else if (a instanceof Long v) {
                enc[2 * i] = 1;
                enc[2 * i + 1] = v;
            } else if (a instanceof Float v) {
                enc[2 * i] = 2;
                enc[2 * i + 1] = WeaveCaller.fromFloat(v);
            } else if (a instanceof Double v) {
                enc[2 * i] = 3;
                enc[2 * i + 1] = WeaveCaller.fromDouble(v);
            } else {
                throw new IllegalArgumentException("unsupported argument type: " + a);
            }
        }
        return WeaveResult.decode(WeaveNative.instanceCall(h(), entry, enc));
    }

    /** Continues an unwound guest (after {@link #snapshot()} if desired). */
    public WeaveResult resume() {
        return WeaveResult.decode(WeaveNative.instanceResume(h()));
    }

    /** The next call/resume unwinds at its next poll. */
    public void unwindNext() {
        WeaveNative.instanceUnwindNext(h());
    }

    /** The next call/resume unwinds after {@code polls} polls (mid-flight checkpoint). */
    public void unwindAfter(long polls) {
        WeaveNative.instanceUnwindAfter(h(), polls);
    }

    /** Captures the whole guest state (only while paused/unwound); gzip-compressed. */
    public byte[] snapshot() {
        byte[] raw = WeaveNative.instanceCheckpoint(h());
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        try (GZIPOutputStream gz = new GZIPOutputStream(out)) {
            gz.write(raw);
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
        return out.toByteArray();
    }

    public State state() {
        return State.values()[WeaveNative.instanceState(h())];
    }

    /** Reads the guest's primary memory. */
    public byte[] readMemory(long offset, int len) {
        return WeaveNative.instanceReadMemory(h(), offset, len);
    }

    /** Writes the guest's primary memory. */
    public void writeMemory(long offset, byte[] data) {
        WeaveNative.instanceWriteMemory(h(), offset, WeaveNative.direct(data));
    }

    /** Thread-safe handle that makes the guest unwind at its next poll. */
    public Cancellation cancellation() {
        return new Cancellation(WeaveNative.cancelCreate(h()));
    }

    @Override
    public void close() {
        if (handle != 0) {
            WeaveNative.instanceClose(handle);
            handle = 0;
        }
    }

    /** Cross-thread cancellation request. Must be closed before the instance is. */
    public static final class Cancellation implements AutoCloseable {
        private long handle;

        private Cancellation(long handle) {
            this.handle = handle;
        }

        public void cancel() {
            if (handle != 0) {
                WeaveNative.cancelRequest(handle);
            }
        }

        @Override
        public void close() {
            if (handle != 0) {
                WeaveNative.cancelClose(handle);
                handle = 0;
            }
        }
    }

    /** Fluent builder for host imports. */
    public static final class Builder {
        private final WeaveModule module;
        private final List<WeaveImport> imports = new ArrayList<>();

        Builder(WeaveModule module) {
            this.module = module;
        }

        /**
         * Defines a host function. {@code signature} is {@code params>results} using
         * {@code i} (i32), {@code l} (i64), {@code f} (f32), {@code d} (f64),
         * e.g. {@code "ii>i"} or {@code "i>"}.
         */
        public Builder define(String moduleName, String name, String signature, WeaveHostFunction fn) {
            int sep = signature.indexOf('>');
            if (sep < 0) {
                throw new IllegalArgumentException("signature must contain '>': " + signature);
            }
            return define(moduleName, name, types(signature.substring(0, sep)),
                    types(signature.substring(sep + 1)), fn);
        }

        public Builder define(String moduleName, String name, List<ValType> params, List<ValType> results,
                WeaveHostFunction fn) {
            imports.add(new WeaveImport(moduleName, name, params, results, fn));
            return this;
        }

        /** Creates a fresh instance (runs the guest's start function). */
        public WeaveInstance build() {
            return new WeaveInstance(WeaveNative.instanceCreateFresh(
                    module.engine().getEnginePtr(), module.handle(), imports));
        }

        /** Creates an instance from a snapshot taken by {@link WeaveInstance#snapshot()}. */
        public WeaveInstance restore(byte[] snapshot) {
            byte[] raw;
            try (GZIPInputStream gz = new GZIPInputStream(new ByteArrayInputStream(snapshot))) {
                raw = gz.readAllBytes();
            } catch (IOException e) {
                throw new UncheckedIOException(e);
            }
            return new WeaveInstance(WeaveNative.instanceCreateRestored(
                    module.engine().getEnginePtr(), module.handle(), imports, WeaveNative.direct(raw)));
        }

        private static List<ValType> types(String s) {
            List<ValType> out = new ArrayList<>();
            for (char c : s.toCharArray()) {
                out.add(switch (c) {
                    case 'i' -> ValType.I32;
                    case 'l' -> ValType.I64;
                    case 'f' -> ValType.F32;
                    case 'd' -> ValType.F64;
                    default -> throw new IllegalArgumentException("unknown type char '" + c + "'");
                });
            }
            return out;
        }
    }
}
