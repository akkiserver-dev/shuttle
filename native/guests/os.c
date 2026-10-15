/* os.c - minimal kernel: load one child program and serve its system calls. */
#include "shuttle.h"

SHUTTLE_ABI_DECLARE();

#define CLASS_STORAGE 0x03
#define DISK_SIZE 0
#define DISK_READ 1

/* system call numbers (defined by this OS, not by the ABI) */
#define SYS_WRITE 1  /* (ptr, len) -> len */
#define SYS_TIME  2  /* () -> tick */
#define SYS_EXIT  3  /* (code) */
#define SYS_SLEEP 4  /* (ticks) */

static u8 prog_buf[256 * 1024];
static char line[512];

static i32 append(i32 n, const char *s) {
    while (*s && n < (i32)sizeof line - 1) line[n++] = *s++;
    return n;
}
static void log_num(const char *prefix, i64 v) {
    i32 n = append(0, prefix);
    n += sh_itoa(v, line + n);
    fw_log_raw(line, n);
}

static i64 disk_size(i32 slot) {
    u64 size = 0;
    i32 r = fw_dev_call(slot, DISK_SIZE, 0, 0, &size, 8);
    return r == 8 ? (i64)size : r;
}
static i32 disk_read(i32 slot, u64 off, void *dst, u32 len) {
    u8 args[12];
    for (i32 i = 0; i < 8; i++) args[i] = (u8)(off >> (8 * i));
    for (i32 i = 0; i < 4; i++) args[8 + i] = (u8)(len >> (8 * i));
    return fw_dev_call(slot, DISK_READ, args, 12, dst, (i32)len);
}

/* Load the first Wasm program found on a storage device other than the boot disk. */
static i32 load_program(i32 boot_slot) {
    i32 slots = fw_dev_slots();
    for (i32 slot = 0; slot < slots; slot++) {
        if (slot == boot_slot) continue;
        u8 info[24];
        if (fw_dev_info(slot, info, sizeof info) != 24) continue;
        if ((info[0] | (info[1] << 8)) != CLASS_STORAGE) continue;
        i64 size = disk_size(slot);
        if (size < 8 || size > (i64)sizeof prog_buf) continue;
        if (disk_read(slot, 0, prog_buf, (u32)size) != size) continue;
        log_num("OS: loading program from slot ", slot);
        return fw_image_load(prog_buf, (i32)size);
    }
    return E_NODEV;
}

void _start(void) {
    u8 args[20];
    i32 boot_slot = -1;
    if (fw_boot_args(args, sizeof args) == 20)
        boot_slot = (i32)(args[0] | (args[1] << 8) | (args[2] << 16) | ((u32)args[3] << 24));
    log_num("OS: kernel started, boot slot = ", boot_slot);

    /* self-test: a dev_call result larger than the buffer is held, not lost */
    {
        u64 size = 0;
        i32 need = fw_dev_call(boot_slot, DISK_SIZE, 0, 0, &size, 4);
        i32 got = fw_dev_result(DEV_HELD_RESULT, &size, 8);
        i32 again = fw_dev_result(DEV_HELD_RESULT, &size, 8);
        i32 n = append(0, "OS: held result: need ");
        n += sh_itoa(need, line + n);
        n = append(n, ", fetched ");
        n += sh_itoa(got, line + n);
        n = append(n, " (disk size ");
        n += sh_itoa((i64)size, line + n);
        n = append(n, "), second fetch ");
        n += sh_itoa(again, line + n);
        fw_log_raw(line, n);
    }

    i32 image = load_program(boot_slot);
    if (image < 0) { log_num("OS: no program: ", image); fw_shutdown(); }

    /* self-test: reservations beyond the CPU's memory are refused */
    log_num("OS: co_create with 1 GiB -> ", co_create(image, 1024LL * 1024 * 1024));

    i32 child = co_create(image, 2 * 1024 * 1024);
    if (child < 0) { log_num("OS: co_create failed: ", child); fw_shutdown(); }
    log_num("OS: started child, handle = ", child);

    co_info info;
    i64 retval = 0;
    i64 total_used = 0;
    for (;;) {
        i32 why = co_resume(child, 400, retval, &info);
        total_used += info.used;
        retval = 0;
        if (why == CO_YIELD) continue;           /* budget used up: keep going */
        if (why == CO_DONE) { fw_log("OS: child returned"); break; }
        if (why == CO_DEAD) { log_num("OS: child trapped, kind = ", info.code); break; }
        if (why < 0)        { log_num("OS: co_resume failed: ", why); break; }

        /* CO_CALL */
        switch (info.code) {
        case SYS_WRITE: {
            i32 len = (i32)info.a[1];
            if (len > 400) len = 400;
            i32 n = append(0, "[child] ");
            if (co_peek(child, (i32)info.a[0], line + n, len) == len) {
                fw_log_raw(line, n + len);
                retval = len;
            } else {
                retval = E_FAULT;
            }
            break;
        }
        case SYS_TIME:
            retval = fw_time();
            break;
        case SYS_SLEEP:
            /* single-tasking kernel: just halt the whole CPU */
            fw_hlt(info.a[0]);
            break;
        case SYS_EXIT:
            log_num("OS: child exited with code ", info.a[0]);
            co_close(child);
            log_num("OS: budget used by child = ", total_used);
            fw_log("OS: shutting down");
            fw_shutdown();
            break;
        default:
            retval = E_INVAL;
        }
    }
    fw_shutdown();
}
