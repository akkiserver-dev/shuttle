package io.github.stefanrichterhuber.wasmtimejavang;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.ArrayList;
import java.util.HexFormat;
import java.util.List;

import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

/**
 * Tests for the Weave (checkpointable wasm) API.
 *
 * The guest is hand-assembled so no wasm toolchain is needed. Equivalent WAT:
 *
 * <pre>
 * (module
 *   (import "sys" "log" (func $log (param i32)))
 *   (memory (export "memory") 1)
 *   (func (export "run") (param $n i32) (result i32)
 *     (local $i i32)
 *     (loop $l
 *       (call $log (local.get $i))
 *       (local.set $i (i32.add (local.get $i) (i32.const 1)))
 *       (br_if $l (i32.lt_u (local.get $i) (local.get $n))))
 *     (local.get $i)))
 * </pre>
 */
class WeaveTest {
    private static final byte[] GUEST = HexFormat.of().parseHex(String.join("",
            "0061736d", "01000000", // magic, version
            "010a", "02", "60017f00", "60017f017f", // types: (i32)->(), (i32)->(i32)
            "020b", "01", "03737973", "036c6f67", "0000", // import sys.log (type 0)
            "0302", "01", "01", // function 1 has type 1
            "0503", "01", "0001", // memory: 1 page
            "0710", "02", "066d656d6f727902" + "00", "0372756e00" + "01", // exports: memory, run (func 1)
            "0a1d", "01", "1b", "01017f", // code: 1 body, 27 bytes, 1 local i32
            "0340", "2001", "1000", "2001", "4101", "6a", "2101", // loop: log(i); i += 1
            "2001", "2000", "49", "0d00", "0b", // br_if i < n
            "2001", "0b")); // return i

    private static WasmtimeEngine engine;
    private static WeaveModule module;

    @BeforeAll
    static void setUp() {
        engine = new WasmtimeEngine();
        module = WeaveModule.load(engine, GUEST, 1, 0); // poll on every back-edge so tiny loops can unwind
    }

    @AfterAll
    static void tearDown() throws Exception {
        module.close();
        engine.close();
    }

    private static WeaveInstance.Builder builder(List<Integer> log) {
        return module.instance().define("sys", "log", "i>", (caller, a) -> {
            log.add((int) a[0]);
            return null;
        });
    }

    @Test
    void runsToCompletion() {
        List<Integer> log = new ArrayList<>();
        try (WeaveInstance vm = builder(log).build()) {
            WeaveResult r = vm.call("run", 5);
            assertTrue(r.isDone());
            assertEquals(5, r.intValue());
            assertEquals(List.of(0, 1, 2, 3, 4), log);
            assertEquals(WeaveInstance.State.COMPLETED, vm.state());
        }
    }

    @Test
    void unwindThenResumeContinuesWithoutRepeatingWork() {
        List<Integer> log = new ArrayList<>();
        try (WeaveInstance vm = builder(log).build()) {
            vm.unwindAfter(3);
            WeaveResult first = vm.call("run", 5);
            assertTrue(first.isUnwound());
            assertEquals(WeaveInstance.State.PAUSED, vm.state());
            assertTrue(log.size() < 5);

            WeaveResult second = vm.resume();
            assertTrue(second.isDone());
            assertEquals(5, second.intValue());
            assertEquals(List.of(0, 1, 2, 3, 4), log);
        }
    }

    @Test
    void snapshotRestoresIntoANewInstanceAndContinues() {
        List<Integer> before = new ArrayList<>();
        byte[] snapshot;
        try (WeaveInstance vm = builder(before).build()) {
            vm.unwindAfter(3);
            assertTrue(vm.call("run", 5).isUnwound());
            snapshot = vm.snapshot();
        }
        assertTrue(snapshot.length > 0);
        assertTrue(before.size() < 5);

        List<Integer> after = new ArrayList<>();
        try (WeaveInstance restored = builder(after).restore(snapshot)) {
            WeaveResult r = restored.resume();
            assertTrue(r.isDone());
            assertEquals(5, r.intValue());
        }

        List<Integer> all = new ArrayList<>(before);
        all.addAll(after);
        assertEquals(List.of(0, 1, 2, 3, 4), all);
    }

    @Test
    void sameSnapshotCanBeRestoredTwice() {
        byte[] snapshot;
        try (WeaveInstance vm = builder(new ArrayList<>()).build()) {
            vm.unwindAfter(2);
            assertTrue(vm.call("run", 4).isUnwound());
            snapshot = vm.snapshot();
        }
        List<Integer> a = new ArrayList<>();
        List<Integer> b = new ArrayList<>();
        try (WeaveInstance x = builder(a).restore(snapshot); WeaveInstance y = builder(b).restore(snapshot)) {
            assertEquals(4, x.resume().intValue());
            assertEquals(4, y.resume().intValue());
        }
        assertEquals(a, b);
    }

    @Test
    void cancellationUnwindsAtNextPoll() {
        try (WeaveInstance vm = builder(new ArrayList<>()).build();
                WeaveInstance.Cancellation cancel = vm.cancellation()) {
            cancel.cancel();
            WeaveResult r = vm.call("run", 5);
            assertTrue(r.isUnwound());
            assertFalse(r.isDone());
            assertEquals(5, vm.resume().intValue());
        }
    }

    @Test
    void hostFunctionCanReadAndWriteGuestMemory() {
        byte[][] seen = new byte[1][];
        try (WeaveInstance vm = module.instance().define("sys", "log", "i>", (caller, a) -> {
            caller.write(16, new byte[] { 1, 2, 3 });
            seen[0] = caller.read(16, 3);
            return null;
        }).build()) {
            vm.call("run", 1);
            assertArrayEquals(new byte[] { 1, 2, 3 }, seen[0]);
            assertArrayEquals(new byte[] { 1, 2, 3 }, vm.readMemory(16, 3));
        }
    }

    @Test
    void instanceMemoryCanBeWrittenFromJava() {
        try (WeaveInstance vm = builder(new ArrayList<>()).build()) {
            vm.writeMemory(100, new byte[] { 9, 8, 7 });
            assertArrayEquals(new byte[] { 9, 8, 7 }, vm.readMemory(100, 3));
        }
    }

    @Test
    void callerIsInvalidOutsideTheHostCall() {
        WeaveCaller[] leaked = new WeaveCaller[1];
        try (WeaveInstance vm = module.instance().define("sys", "log", "i>", (caller, a) -> {
            leaked[0] = caller;
            return null;
        }).build()) {
            vm.call("run", 1);
        }
        assertThrows(IllegalStateException.class, () -> leaked[0].read(0, 1));
    }

    @Test
    void hostFunctionResultCountMustMatchDeclaration() {
        try (WeaveInstance vm = module.instance()
                .define("sys", "log", "i>", (caller, a) -> new long[] { 1 }) // declared void
                .build()) {
            assertThrows(WasmRuntimeException.class, () -> vm.call("run", 1));
        }
    }

    @Test
    void invalidArgumentsAndSignaturesAreRejected() {
        assertThrows(IllegalArgumentException.class,
                () -> module.instance().define("sys", "x", "ii", (c, a) -> null));
        assertThrows(IllegalArgumentException.class,
                () -> module.instance().define("sys", "x", "q>i", (c, a) -> null));
        try (WeaveInstance vm = builder(new ArrayList<>()).build()) {
            assertThrows(IllegalArgumentException.class, () -> vm.call("run", "five"));
        }
    }

    @Test
    void closedInstanceCannotBeUsed() {
        WeaveInstance vm = builder(new ArrayList<>()).build();
        vm.close();
        vm.close(); // idempotent
        assertThrows(IllegalStateException.class, () -> vm.call("run", 1));
    }

    @Test
    void floatAndDoubleBitHelpersRoundTrip() {
        assertEquals(1.5f, WeaveCaller.asFloat(WeaveCaller.fromFloat(1.5f)));
        assertEquals(-2.25, WeaveCaller.asDouble(WeaveCaller.fromDouble(-2.25)));
        assertEquals(0L, WeaveCaller.fromFloat(0f));
    }
}
