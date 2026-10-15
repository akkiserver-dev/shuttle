/* bios.c - minimal BIOS: find a bootable disk, load the OS image, exec it. */
#include "shuttle.h"

SHUTTLE_ABI_DECLARE();

#define CLASS_STORAGE 0x03
#define DISK_SIZE 0  /* () -> u64 */
#define DISK_READ 1  /* (u64 off, u32 len) -> bytes */

static u8 os_buf[512 * 1024];

static void log_num(const char *prefix, i64 v) {
    char line[96];
    i32 n = 0;
    while (prefix[n]) { line[n] = prefix[n]; n++; }
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

void _start(void) {
    char line[96];
    fw_log("BIOS: power on");
    /* "BIOS: CPU <vendor> <model>" */
    i32 n = 0;
    const char *p = "BIOS: CPU ";
    while (*p) line[n++] = *p++;
    i32 v = fw_cpuid(0, line + n, 40);
    if (v > 0 && v <= 40) n += v;
    line[n++] = ' ';
    i32 m = fw_cpuid(1, line + n, 40);
    if (m > 0 && m <= 40) n += m;
    fw_log_raw(line, n);

    i32 slots = fw_dev_slots();
    for (i32 slot = 0; slot < slots; slot++) {
        u8 info[24];
        if (fw_dev_info(slot, info, sizeof info) != 24) continue;
        u16 cls = (u16)(info[0] | (info[1] << 8));
        if (cls != CLASS_STORAGE) continue;

        i64 size = disk_size(slot);
        if (size < 8 || size > (i64)sizeof os_buf) continue;
        if (disk_read(slot, 0, os_buf, (u32)size) != size) continue;
        /* bootable = a Wasm module on the disk */
        if (os_buf[0] != 0 || os_buf[1] != 'a' || os_buf[2] != 's' || os_buf[3] != 'm') continue;

        log_num("BIOS: booting from slot ", slot);
        i32 image = fw_image_load(os_buf, (i32)size);
        if (image < 0) { log_num("BIOS: image_load failed: ", image); continue; }

        /* boot args: the boot slot (u32) followed by the disk address (16 bytes) */
        u8 args[20];
        for (i32 i = 0; i < 4; i++) args[i] = (u8)((u32)slot >> (8 * i));
        for (i32 i = 0; i < 16; i++) args[4 + i] = info[8 + i];
        i32 r = fw_exec(image, args, sizeof args);
        log_num("BIOS: exec failed: ", r);
    }
    fw_log("BIOS: no bootable disk");
    fw_shutdown();
}
