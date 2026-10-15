package jp.akkiserver.shuttle;

/**
 * Handle to the guest during a host call: read/write guest memory, request a
 * checkpoint unwind. Only valid while the host function runs.
 */
public final class WeaveCaller {
    private long ptr;

    // Called from native code.
    WeaveCaller(long ptr) {
        this.ptr = ptr;
    }

    void invalidate() {
        this.ptr = 0;
    }

    private long ptr() {
        if (ptr == 0) {
            throw new IllegalStateException("WeaveCaller is only valid during the host call");
        }
        return ptr;
    }

    /** Reads {@code len} bytes of the guest's primary memory. */
    public byte[] read(long offset, int len) {
        return WeaveNative.callerReadMemory(ptr(), offset, len);
    }

    /** Writes into the guest's primary memory. */
    public void write(long offset, byte[] data) {
        WeaveNative.callerWriteMemory(ptr(), offset, WeaveNative.direct(data));
    }

    /** Reads a UTF-8 string of {@code len} bytes. */
    public String readString(long offset, int len) {
        return new String(read(offset, len), java.nio.charset.StandardCharsets.UTF_8);
    }

    /**
     * Makes the guest unwind at its next poll (VM-exit style yield). The current
     * {@code WeaveInstance.call}/{@code resume} then returns an unwound result.
     */
    public void requestUnwind() {
        WeaveNative.callerRequestUnwind(ptr());
    }

    public static float asFloat(long bits) {
        return Float.intBitsToFloat((int) bits);
    }

    public static double asDouble(long bits) {
        return Double.longBitsToDouble(bits);
    }

    public static long fromFloat(float v) {
        return Float.floatToRawIntBits(v) & 0xFFFFFFFFL;
    }

    public static long fromDouble(double v) {
        return Double.doubleToRawLongBits(v);
    }
}
