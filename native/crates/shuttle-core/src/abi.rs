//! Constants of the firmware ABI draft 0.1 and load-time checks.

use crate::CpuSpec;
use wasmparser::{Parser, Payload, Validator, WasmFeatures};

pub const ABI_MAJOR: u32 = 0;
pub const ABI_MINOR: u32 = 1;
pub const ABI_SECTION: &str = "shuttle.abi";

/// Imports that stop the guest. Weave turns their call sites into unwind
/// points; on resume the call runs again and returns the result.
pub const YIELDING_IMPORTS: &[(&str, &str)] = &[
    ("fw", "hlt"),
    ("fw", "yield"),
    ("fw", "exec"),
    ("fw", "shutdown"),
    ("fw", "reboot"),
    ("sys", "call"),
];

/// `dev_result` ticket that fetches a synchronous `dev_call` result which
/// did not fit the caller's buffer.
pub const HELD_RESULT: i32 = -1;

pub const E_INVAL: i32 = -1;
pub const E_FAULT: i32 = -2;
pub const E_NODEV: i32 = -3;
pub const E_NOMETHOD: i32 = -4;
pub const E_BADHANDLE: i32 = -5;
pub const E_NOMEM: i32 = -6;
pub const E_UNSUPPORTED: i32 = -7;
pub const E_VERSION: i32 = -8;
pub const E_AGAIN: i32 = -9;
pub const E_NOPOWER: i32 = -10;
pub const E_IO: i32 = -11;

pub const CO_YIELD: i32 = 0;
pub const CO_CALL: i32 = 1;
pub const CO_DONE: i32 = 2;
pub const CO_DEAD: i32 = 3;

pub const STATUS_SUSPENDED: i32 = 0;
pub const STATUS_DONE: i32 = 1;
pub const STATUS_DEAD: i32 = 2;

pub const EV_ATTACH: u16 = 1;
pub const EV_DETACH: u16 = 2;
pub const EV_COMPLETE: u16 = 3;
pub const EV_SIGNAL: u16 = 4;
pub const EV_POWER: u16 = 5;
pub const EV_OVERFLOW: u16 = 6;

pub const EVENT_QUEUE_MAX: usize = 256;
pub const EXEC_ARGS_MAX: usize = 4096;
pub const ASYNC_MAX: usize = 16;
pub const CPUID_STR_MAX: usize = 64;
pub const LOG_LINES_PER_TICK: usize = 64;
pub const LOG_LINE_MAX: usize = 1024;

/// The `shuttle.abi` custom section must exist and be compatible.
pub fn check_abi_section(raw: &[u8]) -> Result<(), i32> {
    let mut found = None;
    for payload in Parser::new(0).parse_all(raw) {
        match payload.map_err(|_| E_INVAL)? {
            Payload::CustomSection(c) if c.name() == ABI_SECTION => {
                let d = c.data();
                if d.len() != 8 {
                    return Err(E_VERSION);
                }
                let major = u32::from_le_bytes(d[0..4].try_into().unwrap());
                let minor = u32::from_le_bytes(d[4..8].try_into().unwrap());
                found = Some((major, minor));
            }
            _ => {}
        }
    }
    match found {
        Some((ABI_MAJOR, minor)) if minor <= ABI_MINOR => Ok(()),
        _ => Err(E_VERSION),
    }
}

/// Reject modules that use Wasm features the CPU does not have.
pub fn check_features(raw: &[u8], cpu: &CpuSpec) -> Result<(), i32> {
    // Always available: the MVP plus the small extensions every toolchain emits.
    let mut f = WasmFeatures::MUTABLE_GLOBAL
        | WasmFeatures::SIGN_EXTENSION
        | WasmFeatures::SATURATING_FLOAT_TO_INT
        | WasmFeatures::MULTI_VALUE
        | WasmFeatures::FLOATS;
    if cpu.has_feature("bulk-memory") {
        f |= WasmFeatures::BULK_MEMORY | WasmFeatures::REFERENCE_TYPES;
    }
    if cpu.has_feature("simd") {
        f |= WasmFeatures::SIMD;
    }
    if Validator::new_with_features(f).validate_all(raw).is_ok() {
        return Ok(());
    }
    // Distinguish "uses a feature this CPU lacks" from "not valid at all".
    if Validator::new_with_features(WasmFeatures::default())
        .validate_all(raw)
        .is_ok()
    {
        Err(E_UNSUPPORTED)
    } else {
        Err(E_INVAL)
    }
}
