/* child.c - a user program: it can only talk to the kernel through sys.call. */
#include "shuttle.h"

SHUTTLE_ABI_DECLARE();

#define SYS_WRITE 1
#define SYS_TIME  2
#define SYS_EXIT  3
#define SYS_SLEEP 4

static char buf[128];

static void print(const char *s, i32 len) { sys_call(SYS_WRITE, (i64)(usize)s, len, 0, 0); }

static i32 append(i32 n, const char *s) {
    while (*s) buf[n++] = *s++;
    return n;
}

/* deliberately recursive so the call stack is deep when a tick ends */
static u64 work(u32 depth, u64 acc) {
    if (depth == 0) {
        /* volatile keeps clang from folding the loop away */
        volatile u64 x = acc;
        for (u32 i = 0; i < 2000000; i++) x = x * 6364136223846793005ULL + 1442695040888963407ULL;
        return x;
    }
    return work(depth - 1, acc ^ depth) + depth;
}

void _start(void) {
    print("Hello from the child program!", 29);
    {
        /* the kernel gave this child 2 MiB (32 pages) in total */
        i32 big = __builtin_wasm_memory_grow(0, 64);
        i32 small = __builtin_wasm_memory_grow(0, 1);
        i32 n = append(0, "memory.grow(64 pages) = ");
        n += sh_itoa(big, buf + n);
        n = append(n, ", memory.grow(1 page) = ");
        n += sh_itoa(small, buf + n);
        print(buf, n);
    }
    u64 acc = 1;
    for (i32 round = 1; round <= 5; round++) {
        acc = work(32, acc);
        i64 now = sys_call(SYS_TIME, 0, 0, 0, 0);
        i32 n = append(0, "round ");
        n += sh_itoa(round, buf + n);
        n = append(n, " at tick ");
        n += sh_itoa(now, buf + n);
        n = append(n, ", acc = ");
        n += sh_itoa((i64)(acc % 1000000007ULL), buf + n);
        print(buf, n);
        sys_call(SYS_SLEEP, 2, 0, 0, 0);
    }
    sys_call(SYS_EXIT, 42, 0, 0, 0);
}
