//! JNI bridge for Weave: woven (checkpointable) wasm modules.
//!
//! Weave owns its own `Store<Ctx>`/`Linker<Ctx>`, so this module is independent of
//! the plain `WasmtimeStore`; only the `Engine` is shared with `WasmtimeEngine`.
//! All values cross the JNI boundary as raw `long` bits (i32/i64 as-is, f32/f64 as
//! their IEEE bit patterns) so no boxing or type_map machinery is needed.

use std::ffi::c_void;
use std::sync::Arc;

use jni::{
    JavaVM, bind_java_type,
    objects::{JByteBuffer, JClass, JList, JPrimitiveArray, JString},
    refs::IntoAuto,
    sys::{jbyte, jint, jlong},
};
use log::debug;
use wasmtime::{Caller, Engine, FuncType, Val, ValType};
use wasmtime::error::{bail, format_err};
use weave_core::snapshot::Snapshot;
use weave_host::MemRead;
use weave_transform::TransformOptions;
use weave_wasmtime::{
    CancellationHandle, Ctx, InstanceState, LinkFn, Poller, WeaveInstance, WeaveModule, WorkResult,
};

use crate::wasmexception::{convert_jvm_error_to_wasmtime_error, handle_wasmtime_error};
use crate::wasmtime_valtype::convert_val_type_enum_list_to_vec;

bind_java_type! {
    rust_type = pub JWeaveCaller,
    java_type = "io.github.stefanrichterhuber.wasmtimejavang.WeaveCaller",
    constructors { fn new(ptr: jlong) },
}

bind_java_type! {
    rust_type = pub JWeaveHostFunction,
    java_type = "io.github.stefanrichterhuber.wasmtimejavang.WeaveHostFunction",
    type_map = {
        JWeaveCaller => "io.github.stefanrichterhuber.wasmtimejavang.WeaveCaller",
    },
    methods { fn call(caller: JWeaveCaller, args: jlong[]) -> jlong[] },
}

bind_java_type! {
    rust_type = pub JWeaveImport,
    java_type = "io.github.stefanrichterhuber.wasmtimejavang.WeaveImport",
    type_map = {
        JWeaveHostFunction => "io.github.stefanrichterhuber.wasmtimejavang.WeaveHostFunction",
    },
    methods {
        fn module() -> JString,
        fn name() -> JString,
        fn params() -> JList,
        fn results() -> JList,
        fn function() -> JWeaveHostFunction,
    },
}

bind_java_type! {
    rust_type = pub JWeaveNative,
    java_type = "io.github.stefanrichterhuber.wasmtimejavang.WeaveNative",
    native_methods {
        extern static fn module_create(raw: JByteBuffer, poll_period: jint, stack_pages: jint) -> jlong,
        extern static fn module_close(handle: jlong),
        extern static fn module_hash(handle: jlong) -> jbyte[],
        extern static fn module_entries(handle: jlong) -> JString,
        extern static fn instance_create_fresh(engine: jlong, module: jlong, imports: JList) -> jlong,
        extern static fn instance_create_restored(engine: jlong, module: jlong, imports: JList, snapshot: JByteBuffer) -> jlong,
        extern static fn instance_close(handle: jlong),
        extern static fn instance_call(handle: jlong, entry: JString, args: jlong[]) -> jlong[],
        extern static fn instance_resume(handle: jlong) -> jlong[],
        extern static fn instance_checkpoint(handle: jlong) -> jbyte[],
        extern static fn instance_state(handle: jlong) -> jint,
        extern static fn instance_unwind_next(handle: jlong),
        extern static fn instance_unwind_after(handle: jlong, polls: jlong),
        extern static fn instance_read_memory(handle: jlong, offset: jlong, len: jint) -> jbyte[],
        extern static fn instance_write_memory(handle: jlong, offset: jlong, data: JByteBuffer),
        extern static fn cancel_create(handle: jlong) -> jlong,
        extern static fn cancel_request(handle: jlong),
        extern static fn cancel_close(handle: jlong),
        extern static fn caller_read_memory(caller: jlong, offset: jlong, len: jint) -> jbyte[],
        extern static fn caller_write_memory(caller: jlong, offset: jlong, data: JByteBuffer),
        extern static fn caller_request_unwind(caller: jlong),
    },
}

// ---------------------------------------------------------------- helpers

/// Throws a pending Java exception for `Err` and returns `$default` from the native.
/// The error is passed through unchanged so a Java exception raised inside a host
/// function survives as the cause of the resulting `WasmRuntimeException`.
macro_rules! tri {
    ($env:expr, $e:expr, $default:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => {
                handle_wasmtime_error($env, e)?;
                return Ok($default);
            }
        }
    };
}

fn buf_slice<'a>(
    env: &mut jni::Env<'_>,
    buf: &JByteBuffer<'_>,
) -> Result<&'a [u8], jni::errors::Error> {
    let addr = env.get_direct_buffer_address(buf)?;
    let len = env.get_direct_buffer_capacity(buf)?;
    Ok(unsafe { core::slice::from_raw_parts(addr, len) })
}

fn to_jbytes<'l>(
    env: &mut jni::Env<'l>,
    v: &[u8],
) -> Result<JPrimitiveArray<'l, jbyte>, jni::errors::Error> {
    let arr = env.new_byte_array(v.len())?;
    let s: &[i8] = unsafe { core::slice::from_raw_parts(v.as_ptr() as *const i8, v.len()) };
    arr.set_region(env, 0, s)?;
    Ok(arr)
}

fn to_jlongs<'l>(
    env: &mut jni::Env<'l>,
    v: &[jlong],
) -> Result<JPrimitiveArray<'l, jlong>, jni::errors::Error> {
    let arr = env.new_long_array(v.len())?;
    arr.set_region(env, 0, v)?;
    Ok(arr)
}

fn from_jlongs(
    env: &mut jni::Env<'_>,
    arr: &JPrimitiveArray<'_, jlong>,
) -> Result<Vec<jlong>, jni::errors::Error> {
    let n = arr.len(env)?;
    let mut buf = vec![0 as jlong; n];
    arr.get_region(env, 0, &mut buf)?;
    Ok(buf)
}

fn val_to_bits(v: &Val) -> wasmtime::error::Result<(jlong, jlong)> {
    Ok(match v {
        Val::I32(x) => (0, *x as jlong),
        Val::I64(x) => (1, *x),
        Val::F32(b) => (2, *b as jlong),
        Val::F64(b) => (3, *b as jlong),
        other => bail!("unsupported value type crossing the Weave boundary: {other:?}"),
    })
}

fn bits_to_val(kind: jlong, bits: jlong) -> wasmtime::error::Result<Val> {
    Ok(match kind {
        0 => Val::I32(bits as i32),
        1 => Val::I64(bits),
        2 => Val::F32(bits as u32),
        3 => Val::F64(bits as u64),
        k => bail!("unknown value kind {k}"),
    })
}

fn bits_to_typed_val(ty: &ValType, bits: jlong) -> wasmtime::error::Result<Val> {
    Ok(match ty {
        ValType::I32 => Val::I32(bits as i32),
        ValType::I64 => Val::I64(bits),
        ValType::F32 => Val::F32(bits as u32),
        ValType::F64 => Val::F64(bits as u64),
        other => bail!("unsupported host function type {other:?} (only i32/i64/f32/f64)"),
    })
}

/// Layout: [status, n, kind0, bits0, kind1, bits1, ...]; status 0 = done, 1 = unwound.
fn encode_result(r: WorkResult) -> wasmtime::error::Result<Vec<jlong>> {
    match r {
        WorkResult::Unwound => Ok(vec![1, 0]),
        WorkResult::Done(vals) => {
            let mut out = vec![0, vals.len() as jlong];
            for v in &vals {
                let (k, b) = val_to_bits(v)?;
                out.push(k);
                out.push(b);
            }
            Ok(out)
        }
    }
}

unsafe fn inst<'a>(h: jlong) -> &'a mut WeaveInstance {
    unsafe { &mut *(h as *mut WeaveInstance) }
}

/// Unwind requests are one-shot: weave leaves `UnwindNext`/`UnwindAfter(0)` installed
/// after the unwind, which would make the following `resume` unwind again immediately.
fn finish_run(instance: &mut WeaveInstance, r: wasmtime::error::Result<WorkResult>) -> wasmtime::error::Result<WorkResult> {
    if matches!(r, Ok(WorkResult::Unwound)) {
        instance.set_poller(Poller::Run);
    }
    r
}

/// Primary memory of the guest seen through a host-call `Caller`.
fn caller_memory(caller: &mut Caller<'static, Ctx>) -> wasmtime::error::Result<wasmtime::Memory> {
    let name = caller
        .data()
        .mem_names
        .first()
        .cloned()
        .ok_or_else(|| format_err!("guest has no exported memory"))?;
    caller
        .get_export(&name)
        .and_then(|e| e.into_memory())
        .ok_or_else(|| format_err!("guest memory export {name} not found"))
}

unsafe fn caller<'a>(h: jlong) -> Option<&'a mut Caller<'static, Ctx>> {
    if h == 0 {
        None
    } else {
        Some(unsafe { &mut *(h as *mut Caller<'static, Ctx>) })
    }
}

struct ImportDef {
    module: String,
    name: String,
    params: Vec<ValType>,
    results: Vec<ValType>,
    func: Arc<jni::refs::Global<JWeaveHostFunction<'static>>>,
}

fn read_imports(
    env: &mut jni::Env<'_>,
    imports: JList<'_>,
) -> Result<Vec<ImportDef>, jni::errors::Error> {
    let mut defs = Vec::new();
    let it = imports.iter(env)?;
    while let Some(item) = it.next(env)? {
        let item = env.cast_local::<JWeaveImport>(item)?.auto();
        let module = item.module(env)?.to_string();
        let name = item.name(env)?.to_string();
        let params = item.params(env)?;
        let results = item.results(env)?;
        let params = convert_val_type_enum_list_to_vec(env, params)?;
        let results = convert_val_type_enum_list_to_vec(env, results)?;
        let function = item.function(env)?;
        let func = Arc::new(env.new_global_ref(function)?);
        defs.push(ImportDef { module, name, params, results, func });
    }
    Ok(defs)
}

fn make_link(engine: &Engine, jvm: JavaVM, defs: Vec<ImportDef>) -> LinkFn {
    let engine = engine.clone();
    let jvm = Arc::new(jvm);
    Box::new(move |linker| {
        for d in &defs {
            let ty = FuncType::new(&engine, d.params.clone(), d.results.clone());
            let func = d.func.clone();
            let jvm = jvm.clone();
            let results = d.results.clone();
            let label = format!("{}::{}", d.module, d.name);
            linker.func_new(&d.module, &d.name, ty, move |mut caller: Caller<'_, Ctx>, args, rets| {
                let bits: Vec<jlong> = args
                    .iter()
                    .map(|v| val_to_bits(v).map(|(_, b)| b))
                    .collect::<wasmtime::error::Result<_>>()?;
                // Java may call back into `caller_*` natives while this frame is alive.
                // `caller` is not touched again until Java returns.
                let ptr = &mut caller as *mut Caller<'_, Ctx> as *mut c_void as jlong;
                let out: Result<Vec<jlong>, jni::errors::Error> = jvm.attach_current_thread(|env| {
                    let caller_obj = JWeaveCaller::new(env, ptr)?;
                    let arr = to_jlongs(env, &bits)?;
                    let res = func.call(env, &caller_obj, &arr)?;
                    from_jlongs(env, &res)
                });
                let out = out.map_err(|e| {
                    debug!("Weave host function {label} failed: {e}");
                    convert_jvm_error_to_wasmtime_error(e)
                })?;
                if out.len() != results.len() {
                    bail!("host function {label} returned {} values, expected {}", out.len(), results.len());
                }
                for (i, ty) in results.iter().enumerate() {
                    rets[i] = bits_to_typed_val(ty, out[i])?;
                }
                Ok(())
            })?;
        }
        Ok(())
    })
}

// ---------------------------------------------------------------- natives

impl JWeaveNativeNativeInterface for JWeaveNativeAPI {
    type Error = jni::errors::Error;

    fn module_create<'local>(
        env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        raw: JByteBuffer<'local>,
        poll_period: jint,
        stack_pages: jint,
    ) -> Result<jlong, Self::Error> {
        let bytes = buf_slice(env, &raw)?;
        // Accept both the binary format and the WAT text format.
        let wasm = tri!(env, wat::parse_bytes(bytes).map_err(|e| format_err!("{e}")), 0);
        let defaults = TransformOptions::default();
        let opts = TransformOptions {
            poll_period: if poll_period > 0 { poll_period as u32 } else { defaults.poll_period },
            stack_pages: if stack_pages > 0 { stack_pages as u32 } else { defaults.stack_pages },
        };
        let module = tri!(env, WeaveModule::from_raw(&wasm, &opts), 0);
        Ok(Box::into_raw(Box::new(module)) as jlong)
    }

    fn module_hash<'local>(
        env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        handle: jlong,
    ) -> Result<JPrimitiveArray<'local, jbyte>, Self::Error> {
        let module = unsafe { &*(handle as *const WeaveModule) };
        to_jbytes(env, &module.module_hash)
    }

    /// Entries as `name\tparams>results` lines, using the `WeaveInstance.Builder` type chars.
    fn module_entries<'local>(
        env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        handle: jlong,
    ) -> Result<JString<'local>, Self::Error> {
        use weave_core::ValType as T;
        let module = unsafe { &*(handle as *const WeaveModule) };
        let sig = |types: &[T]| -> String {
            types
                .iter()
                .map(|t| match t {
                    T::I32 => 'i',
                    T::I64 => 'l',
                    T::F32 => 'f',
                    T::F64 => 'd',
                    T::V128 => 'v',
                    T::FuncRef => 'r',
                })
                .collect()
        };
        let lines: Vec<String> = module
            .meta
            .entries
            .iter()
            .map(|e| format!("{}\t{}>{}", e.name, sig(&e.params), sig(&e.results)))
            .collect();
        JString::from_str(env, lines.join("\n"))
    }

    fn module_close<'local>(
        _env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        handle: jlong,
    ) -> Result<(), Self::Error> {
        if handle != 0 {
            drop(unsafe { Box::from_raw(handle as *mut WeaveModule) });
        }
        Ok(())
    }

    fn instance_create_fresh<'local>(
        env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        engine: jlong,
        module: jlong,
        imports: JList<'local>,
    ) -> Result<jlong, Self::Error> {
        let engine = unsafe { &*(engine as *const Engine) };
        let module = unsafe { &*(module as *const WeaveModule) };
        let defs = read_imports(env, imports)?;
        let link = make_link(engine, env.get_java_vm()?, defs);
        let instance = tri!(env, WeaveInstance::new_fresh(engine, module, vec![], vec![], link), 0);
        Ok(Box::into_raw(Box::new(instance)) as jlong)
    }

    fn instance_create_restored<'local>(
        env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        engine: jlong,
        module: jlong,
        imports: JList<'local>,
        snapshot: JByteBuffer<'local>,
    ) -> Result<jlong, Self::Error> {
        let engine = unsafe { &*(engine as *const Engine) };
        let module = unsafe { &*(module as *const WeaveModule) };
        let snap = tri!(env, Snapshot::decode(buf_slice(env, &snapshot)?), 0);
        let defs = read_imports(env, imports)?;
        let link = make_link(engine, env.get_java_vm()?, defs);
        let mut instance =
            tri!(env, WeaveInstance::new_restored(engine, module, vec![], vec![], link), 0);
        tri!(env, instance.restore(&snap), 0);
        Ok(Box::into_raw(Box::new(instance)) as jlong)
    }

    fn instance_close<'local>(
        _env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        handle: jlong,
    ) -> Result<(), Self::Error> {
        if handle != 0 {
            drop(unsafe { Box::from_raw(handle as *mut WeaveInstance) });
        }
        Ok(())
    }

    fn instance_call<'local>(
        env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        handle: jlong,
        entry: JString<'local>,
        args: JPrimitiveArray<'local, jlong>,
    ) -> Result<JPrimitiveArray<'local, jlong>, Self::Error> {
        let entry = entry.to_string();
        let raw = from_jlongs(env, &args)?;
        let vals: wasmtime::error::Result<Vec<Val>> =
            raw.chunks(2).map(|c| bits_to_val(c[0], c[1])).collect();
        let vals = tri!(env, vals, JPrimitiveArray::null());
        let instance = unsafe { inst(handle) };
        let r = instance.call_entry(&entry, &vals);
        let r = tri!(env, finish_run(instance, r), JPrimitiveArray::null());
        let enc = tri!(env, encode_result(r), JPrimitiveArray::null());
        to_jlongs(env, &enc)
    }

    fn instance_resume<'local>(
        env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        handle: jlong,
    ) -> Result<JPrimitiveArray<'local, jlong>, Self::Error> {
        let instance = unsafe { inst(handle) };
        let r = instance.resume();
        let r = tri!(env, finish_run(instance, r), JPrimitiveArray::null());
        let enc = tri!(env, encode_result(r), JPrimitiveArray::null());
        to_jlongs(env, &enc)
    }

    fn instance_checkpoint<'local>(
        env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        handle: jlong,
    ) -> Result<JPrimitiveArray<'local, jbyte>, Self::Error> {
        let snap = tri!(env, unsafe { inst(handle) }.checkpoint(), JPrimitiveArray::null());
        to_jbytes(env, &snap.encode())
    }

    fn instance_state<'local>(
        _env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        handle: jlong,
    ) -> Result<jint, Self::Error> {
        Ok(match unsafe { inst(handle) }.state() {
            InstanceState::Ready => 0,
            InstanceState::RestoreTarget => 1,
            InstanceState::Restoring => 2,
            InstanceState::Running => 3,
            InstanceState::Paused => 4,
            InstanceState::Completed => 5,
            InstanceState::Retired => 6,
            InstanceState::Failed => 7,
        })
    }

    fn instance_unwind_next<'local>(
        _env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        handle: jlong,
    ) -> Result<(), Self::Error> {
        unsafe { inst(handle) }.set_poller(Poller::UnwindNext);
        Ok(())
    }

    fn instance_unwind_after<'local>(
        _env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        handle: jlong,
        polls: jlong,
    ) -> Result<(), Self::Error> {
        unsafe { inst(handle) }.set_poller(Poller::UnwindAfter(polls.max(0) as u64));
        Ok(())
    }

    fn instance_read_memory<'local>(
        env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        handle: jlong,
        offset: jlong,
        len: jint,
    ) -> Result<JPrimitiveArray<'local, jbyte>, Self::Error> {
        let instance = unsafe { inst(handle) };
        let view = tri!(env, instance.mem_view(), JPrimitiveArray::null());
        let (off, len) = (offset as usize, len.max(0) as usize);
        if view.n_mems() == 0 || off.checked_add(len).is_none_or(|e| e > view.size(0)) {
            handle_wasmtime_error(env, format_err!("memory read out of bounds"))?;
            return Ok(JPrimitiveArray::null());
        }
        let mut buf = vec![0u8; len];
        view.read(0, off, &mut buf);
        to_jbytes(env, &buf)
    }

    fn instance_write_memory<'local>(
        env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        handle: jlong,
        offset: jlong,
        data: JByteBuffer<'local>,
    ) -> Result<(), Self::Error> {
        let bytes = buf_slice(env, &data)?;
        let instance = unsafe { inst(handle) };
        // `write_mem` would silently grow the memory; growing behind the guest's back
        // can corrupt its allocator, so out-of-range writes are rejected like reads.
        let size = {
            let view = tri!(env, instance.mem_view(), ());
            if view.n_mems() == 0 { 0 } else { view.size(0) }
        };
        let off = offset as usize;
        if offset < 0 || off.checked_add(bytes.len()).is_none_or(|e| e > size) {
            handle_wasmtime_error(env, format_err!("memory write out of bounds"))?;
            return Ok(());
        }
        tri!(env, instance.write_mem(0, off, bytes), ());
        Ok(())
    }

    fn cancel_create<'local>(
        _env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        handle: jlong,
    ) -> Result<jlong, Self::Error> {
        let h = unsafe { inst(handle) }.cancellation_handle();
        Ok(Box::into_raw(Box::new(h)) as jlong)
    }

    fn cancel_request<'local>(
        _env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        handle: jlong,
    ) -> Result<(), Self::Error> {
        unsafe { &*(handle as *const CancellationHandle) }.cancel();
        Ok(())
    }

    fn cancel_close<'local>(
        _env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        handle: jlong,
    ) -> Result<(), Self::Error> {
        if handle != 0 {
            drop(unsafe { Box::from_raw(handle as *mut CancellationHandle) });
        }
        Ok(())
    }

    fn caller_read_memory<'local>(
        env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        caller_ptr: jlong,
        offset: jlong,
        len: jint,
    ) -> Result<JPrimitiveArray<'local, jbyte>, Self::Error> {
        let Some(caller) = (unsafe { caller(caller_ptr) }) else {
            handle_wasmtime_error(env, format_err!("WeaveCaller is no longer valid"))?;
            return Ok(JPrimitiveArray::null());
        };
        let mem = tri!(env, caller_memory(caller), JPrimitiveArray::null());
        let data = mem.data(&*caller);
        let (off, len) = (offset as usize, len.max(0) as usize);
        match off.checked_add(len) {
            Some(end) if end <= data.len() => to_jbytes(env, &data[off..end]),
            _ => {
                handle_wasmtime_error(env, format_err!("memory read out of bounds"))?;
                Ok(JPrimitiveArray::null())
            }
        }
    }

    fn caller_write_memory<'local>(
        env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        caller_ptr: jlong,
        offset: jlong,
        data: JByteBuffer<'local>,
    ) -> Result<(), Self::Error> {
        let bytes = buf_slice(env, &data)?;
        let Some(caller) = (unsafe { caller(caller_ptr) }) else {
            handle_wasmtime_error(env, format_err!("WeaveCaller is no longer valid"))?;
            return Ok(());
        };
        let mem = tri!(env, caller_memory(caller), ());
        let dst = mem.data_mut(&mut *caller);
        let off = offset as usize;
        match off.checked_add(bytes.len()) {
            Some(end) if end <= dst.len() => dst[off..end].copy_from_slice(bytes),
            _ => handle_wasmtime_error(env, format_err!("memory write out of bounds"))?,
        }
        Ok(())
    }

    fn caller_request_unwind<'local>(
        env: &mut jni::Env<'local>,
        _class: JClass<'local>,
        caller_ptr: jlong,
    ) -> Result<(), Self::Error> {
        match unsafe { caller(caller_ptr) } {
            // Takes effect at the next instrumented poll in the guest.
            Some(c) => c.data_mut().poller = Poller::UnwindNext,
            None => handle_wasmtime_error(env, format_err!("WeaveCaller is no longer valid"))?,
        }
        Ok(())
    }
}
