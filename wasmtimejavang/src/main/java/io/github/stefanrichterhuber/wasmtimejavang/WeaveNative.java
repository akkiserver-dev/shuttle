package io.github.stefanrichterhuber.wasmtimejavang;

import java.nio.ByteBuffer;
import java.util.List;

/** Raw native entry points for Weave. Use {@link WeaveModule}/{@link WeaveInstance}. */
final class WeaveNative {
    static {
        // Loads libwasmtimebinding and initializes native logging.
        try {
            Class.forName(WasmtimeEngine.class.getName());
        } catch (ClassNotFoundException e) {
            throw new ExceptionInInitializerError(e);
        }
    }

    private WeaveNative() {
    }

    static ByteBuffer direct(byte[] data) {
        ByteBuffer b = ByteBuffer.allocateDirect(data.length);
        b.put(data).flip();
        return b;
    }

    static native long moduleCreate(ByteBuffer raw, int pollPeriod, int stackPages);

    static native byte[] moduleHash(long handle);

    static native String moduleEntries(long handle);

    static native void moduleClose(long handle);

    static native long instanceCreateFresh(long engine, long module, List<?> imports);

    static native long instanceCreateRestored(long engine, long module, List<?> imports, ByteBuffer snapshot);

    static native void instanceClose(long handle);

    static native long[] instanceCall(long handle, String entry, long[] args);

    static native long[] instanceResume(long handle);

    static native byte[] instanceCheckpoint(long handle);

    static native int instanceState(long handle);

    static native void instanceUnwindNext(long handle);

    static native void instanceUnwindAfter(long handle, long polls);

    static native byte[] instanceReadMemory(long handle, long offset, int len);

    static native void instanceWriteMemory(long handle, long offset, ByteBuffer data);

    static native long cancelCreate(long handle);

    static native void cancelRequest(long handle);

    static native void cancelClose(long handle);

    static native byte[] callerReadMemory(long caller, long offset, int len);

    static native void callerWriteMemory(long caller, long offset, ByteBuffer data);

    static native void callerRequestUnwind(long caller);
}
