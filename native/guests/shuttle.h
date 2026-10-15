/*
 * shuttle.h - Shuttle firmware ABI for C guests (freestanding, wasm32).
 *
 * Draft ABI 0.1. Every program must include SHUTTLE_ABI_DECLARE() exactly once
 * (it emits the `shuttle.abi` custom section the firmware checks at load).
 *
 * Calls that stop the CPU (hlt, yield, exec, shutdown, reboot, sys.call) are
 * plain imports: the firmware registers them as Weave "yielding imports", so
 * the guest is suspended inside the call and the call returns its result when
 * the guest is resumed. No wrappers or hidden imports are needed.
 */
#ifndef SHUTTLE_H
#define SHUTTLE_H

typedef signed char        i8;
typedef unsigned char      u8;
typedef short              i16;
typedef unsigned short     u16;
typedef int                i32;
typedef unsigned int       u32;
typedef long long          i64;
typedef unsigned long long u64;
typedef unsigned long      usize;

#define SHUTTLE_ABI_MAJOR 0
#define SHUTTLE_ABI_MINOR 1

/* Emits the 8-byte `shuttle.abi` custom section: major (u32 LE), minor (u32 LE). */
#define SHUTTLE_STR2(x) #x
#define SHUTTLE_STR(x) SHUTTLE_STR2(x)
#define SHUTTLE_ABI_DECLARE()                                                  \
    __asm__(".section .custom_section.shuttle.abi,\"\",@\n"                    \
            ".int32 " SHUTTLE_STR(SHUTTLE_ABI_MAJOR) "\n"                      \
            ".int32 " SHUTTLE_STR(SHUTTLE_ABI_MINOR) "\n"                      \
            ".text\n")

#define IMPORT(mod, name) __attribute__((import_module(mod), import_name(name)))

/* ---- error codes ---- */
#define E_INVAL       (-1)
#define E_FAULT       (-2)
#define E_NODEV       (-3)
#define E_NOMETHOD    (-4)
#define E_BADHANDLE   (-5)
#define E_NOMEM       (-6)
#define E_UNSUPPORTED (-7)
#define E_VERSION     (-8)
#define E_AGAIN       (-9)
#define E_NOPOWER     (-10)
#define E_IO          (-11)

/* ---- fw: CPU ---- */
IMPORT("fw", "cpuid")  i32  fw_cpuid(i32 leaf, void *out, i32 cap);
IMPORT("fw", "time")   i64  fw_time(void);
IMPORT("fw", "log")    void fw_log_raw(const void *ptr, i32 len);
IMPORT("fw", "hlt")    i32  fw_hlt(i64 timeout);   /* 1 = event, 0 = timeout */
IMPORT("fw", "yield")  void fw_yield(void);
IMPORT("fw", "shutdown") __attribute__((noreturn)) void fw_shutdown(void);
IMPORT("fw", "reboot")   __attribute__((noreturn)) void fw_reboot(void);

/* ---- fw: devices ---- */
IMPORT("fw", "dev_slots")    i32 fw_dev_slots(void);
IMPORT("fw", "dev_info")     i32 fw_dev_info(i32 slot, void *out, i32 cap);
IMPORT("fw", "dev_describe") i32 fw_dev_describe(i32 slot, void *out, i32 cap);
IMPORT("fw", "dev_call")     i32 fw_dev_call(i32 slot, i32 method, const void *args, i32 alen, void *ret, i32 rcap);
IMPORT("fw", "dev_submit")   i32 fw_dev_submit(i32 slot, i32 method, const void *args, i32 alen);
IMPORT("fw", "dev_result")   i32 fw_dev_result(i32 ticket, void *ret, i32 rcap);
/* ticket for a dev_call result that did not fit its buffer */
#define DEV_HELD_RESULT (-1)

/* ---- fw: events ---- */
IMPORT("fw", "poll_event") i32 fw_poll_event(void *out, i32 cap);

/* ---- fw: images ---- */
IMPORT("fw", "image_load") i32  fw_image_load(const void *ptr, i32 len);
IMPORT("fw", "image_free") i32  fw_image_free(i32 image);
IMPORT("fw", "exec")       i32  fw_exec(i32 image, const void *args, i32 alen); /* returns only on failure */
IMPORT("fw", "boot_args")  i32  fw_boot_args(void *out, i32 cap);

/* ---- co: virtualization ---- */
#define CO_YIELD 0
#define CO_CALL  1
#define CO_DONE  2
#define CO_DEAD  3

typedef struct {
    i32 reason;
    i32 code;      /* syscall number (CALL) or trap kind (DEAD) */
    i64 a[4];
    i64 used;
} co_info;

IMPORT("co", "co_create") i32 co_create(i32 image, i64 mem_limit);
IMPORT("co", "co_resume") i32 co_resume(i32 h, i64 budget, i64 retval, co_info *info);
IMPORT("co", "co_status") i32 co_status(i32 h);
IMPORT("co", "co_peek")   i32 co_peek(i32 h, i32 addr, void *buf, i32 len);
IMPORT("co", "co_poke")   i32 co_poke(i32 h, i32 addr, const void *buf, i32 len);
IMPORT("co", "co_close")  i32 co_close(i32 h);

/* ---- sys: the only import a child program sees ---- */
IMPORT("sys", "call") i64 sys_call(i32 no, i64 a0, i64 a1, i64 a2, i64 a3);

/* ---- tiny freestanding helpers ---- */
static inline usize sh_strlen(const char *s) {
    usize n = 0;
    while (s[n]) n++;
    return n;
}
static inline void fw_log(const char *s) { fw_log_raw(s, (i32)sh_strlen(s)); }

/* Write the decimal form of v into buf (at least 21 bytes); returns length. */
static inline i32 sh_itoa(i64 v, char *buf) {
    char tmp[24];
    i32 n = 0, len = 0;
    u64 u = v < 0 ? (u64)(-(v + 1)) + 1 : (u64)v;
    do { tmp[n++] = (char)('0' + u % 10); u /= 10; } while (u);
    if (v < 0) buf[len++] = '-';
    while (n) buf[len++] = tmp[--n];
    buf[len] = 0;
    return len;
}

#endif
