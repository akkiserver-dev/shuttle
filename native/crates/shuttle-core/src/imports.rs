//! Host functions of the firmware ABI: `fw.*` and `co.*` for the top level,
//! `sys.call` for children.
//!
//! Calls that stop the guest (hlt, yield, exec, shutdown, reboot, sys.call)
//! are Weave *yielding imports*: the first call records the request and asks
//! Weave to unwind right after it returns. When the guest is resumed, Weave
//! runs the same call again; that second call returns the result (the
//! `*_reply` fields) without stopping.

use crate::abi::*;
use crate::machine::{AsyncReq, Child, ChildCtl, Exit, KernelCtl};
use wasmtime::{Caller, Linker, Memory};
use weave_core::names;
use weave_host::MemRead;
use weave_wasmtime::{request_unwind, Ctx, LinkFn, Poller, WeaveInstance, WorkResult};

type C<'a> = Caller<'a, Ctx>;

fn ctl<'a>(caller: &'a mut C<'_>) -> &'a mut KernelCtl {
    caller
        .data_mut()
        .service_state::<Option<Box<KernelCtl>>>(0)
        .as_mut()
        .expect("top-level instance without ctl")
}

fn memory(caller: &mut C<'_>) -> Option<Memory> {
    caller.get_export("memory").and_then(|e| e.into_memory())
}

fn read_bytes(caller: &mut C<'_>, ptr: i32, len: i32) -> Result<Vec<u8>, i32> {
    if len < 0 {
        return Err(E_INVAL);
    }
    let mem = memory(caller).ok_or(E_FAULT)?;
    let data = mem.data(&caller);
    let start = ptr as u32 as usize;
    let end = start.checked_add(len as usize).ok_or(E_FAULT)?;
    data.get(start..end).map(|s| s.to_vec()).ok_or(E_FAULT)
}

fn write_bytes(caller: &mut C<'_>, ptr: i32, bytes: &[u8]) -> Result<(), i32> {
    let mem = memory(caller).ok_or(E_FAULT)?;
    let data = mem.data_mut(&mut *caller);
    let start = ptr as u32 as usize;
    let end = start.checked_add(bytes.len()).ok_or(E_FAULT)?;
    data.get_mut(start..end).ok_or(E_FAULT)?.copy_from_slice(bytes);
    Ok(())
}

/// The variable-length output rule: write only if it fits; always return the
/// size needed.
fn write_out(caller: &mut C<'_>, out: i32, cap: i32, bytes: &[u8]) -> i32 {
    if cap < 0 {
        return E_INVAL;
    }
    if bytes.len() <= cap as usize {
        if let Err(e) = write_bytes(caller, out, bytes) {
            return e;
        }
    }
    bytes.len() as i32
}

/// Make the guest unwind as soon as the current (yielding) import returns.
fn request_exit(caller: &mut C<'_>) {
    // Only fails during module initialization, where no exit can be served;
    // the call then simply returns its placeholder.
    let _ = request_unwind(caller);
}

fn err_code(r: Result<i32, i32>) -> i32 {
    match r {
        Ok(v) => v,
        Err(e) => e,
    }
}

// ---------------------------------------------------------------------------

pub(crate) fn kernel_linker() -> LinkFn {
    Box::new(|linker: &mut Linker<Ctx>| {
        link_fw_cpu(linker)?;
        link_fw_dev(linker)?;
        link_fw_image(linker)?;
        link_co(linker)?;
        Ok(())
    })
}

pub(crate) fn child_linker() -> LinkFn {
    Box::new(|linker: &mut Linker<Ctx>| {
        linker.func_wrap(
            "sys",
            "call",
            |mut caller: C<'_>, no: i32, a0: i64, a1: i64, a2: i64, a3: i64| -> i64 {
                let cc = caller.data_mut().service_state::<ChildCtl>(0);
                if let Some(v) = cc.reply.take() {
                    return v; // re-executed after resume: deliver the result
                }
                cc.pending_call = Some((no, [a0, a1, a2, a3]));
                request_exit(&mut caller);
                0
            },
        )?;
        Ok(())
    })
}

fn link_fw_cpu(linker: &mut Linker<Ctx>) -> wasmtime::Result<()> {
    linker.func_wrap("fw", "cpuid", |mut caller: C<'_>, leaf: i32, out: i32, cap: i32| -> i32 {
        let cpu = ctl(&mut caller).cpu.clone();
        let bytes: Vec<u8> = match leaf {
            0 => cpu.vendor.into_bytes(),
            1 => cpu.model.into_bytes(),
            2 => cpu.arch.into_bytes(),
            3 => cpu.features.into_bytes(),
            0x100 => cpu.clock.to_le_bytes().to_vec(),
            0x101 => cpu.max_memory.to_le_bytes().to_vec(),
            _ => return E_INVAL,
        };
        let mut bytes = bytes;
        bytes.truncate(CPUID_STR_MAX);
        write_out(&mut caller, out, cap, &bytes)
    })?;
    linker.func_wrap("fw", "time", |mut caller: C<'_>| -> i64 { ctl(&mut caller).tick as i64 })?;
    linker.func_wrap("fw", "log", |mut caller: C<'_>, ptr: i32, len: i32| {
        let len = len.min(LOG_LINE_MAX as i32);
        if let Ok(bytes) = read_bytes(&mut caller, ptr, len) {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            ctl(&mut caller).log(text);
        }
    })?;
    linker.func_wrap("fw", "hlt", |mut caller: C<'_>, timeout: i64| -> i32 {
        let c = ctl(&mut caller);
        if let Some(v) = c.exit_reply.take() {
            return v as i32; // woken up: 1 = event, 0 = timeout
        }
        if !c.events.is_empty() {
            return 1; // an event is already waiting: do not stop
        }
        c.exit = Some(Exit::Hlt(timeout));
        request_exit(&mut caller);
        0
    })?;
    linker.func_wrap("fw", "yield", |mut caller: C<'_>| {
        let c = ctl(&mut caller);
        if c.exit_reply.take().is_some() {
            return;
        }
        c.exit = Some(Exit::Yield);
        request_exit(&mut caller);
    })?;
    linker.func_wrap("fw", "shutdown", |mut caller: C<'_>| {
        ctl(&mut caller).exit = Some(Exit::Shutdown);
        request_exit(&mut caller);
    })?;
    linker.func_wrap("fw", "reboot", |mut caller: C<'_>| {
        ctl(&mut caller).exit = Some(Exit::Reboot);
        request_exit(&mut caller);
    })?;
    Ok(())
}

fn link_fw_dev(linker: &mut Linker<Ctx>) -> wasmtime::Result<()> {
    linker.func_wrap("fw", "dev_slots", |mut caller: C<'_>| -> i32 {
        ctl(&mut caller).devices.len() as i32
    })?;
    linker.func_wrap("fw", "dev_info", |mut caller: C<'_>, slot: i32, out: i32, cap: i32| -> i32 {
        let info = match ctl(&mut caller).devices.get(slot as usize).and_then(Option::as_ref) {
            Some(d) => d.info(),
            None => return E_NODEV,
        };
        let mut b = Vec::with_capacity(24);
        b.extend_from_slice(&info.class.to_le_bytes());
        b.extend_from_slice(&info.vendor.to_le_bytes());
        b.extend_from_slice(&info.device.to_le_bytes());
        b.extend_from_slice(&info.flags.to_le_bytes());
        b.extend_from_slice(&info.address);
        write_out(&mut caller, out, cap, &b)
    })?;
    linker.func_wrap("fw", "dev_describe", |mut caller: C<'_>, slot: i32, out: i32, cap: i32| -> i32 {
        let desc = match ctl(&mut caller).devices.get(slot as usize).and_then(Option::as_ref) {
            Some(d) => d.describe(),
            None => return E_NODEV,
        };
        write_out(&mut caller, out, cap, &desc)
    })?;
    linker.func_wrap(
        "fw",
        "dev_call",
        |mut caller: C<'_>, slot: i32, method: i32, args: i32, alen: i32, ret: i32, rcap: i32| -> i32 {
            let args = match read_bytes(&mut caller, args, alen) {
                Ok(a) => a,
                Err(e) => return e,
            };
            let result = match ctl(&mut caller).devices.get_mut(slot as usize).and_then(Option::as_mut) {
                Some(d) => d.call(method, &args),
                None => return E_NODEV,
            };
            match result {
                Ok(bytes) => {
                    // The device has already run: a result that does not fit is
                    // kept for dev_result(-1) instead of being thrown away.
                    let n = write_out(&mut caller, ret, rcap, &bytes);
                    let fits = n >= 0 && (n as usize) <= rcap.max(0) as usize;
                    ctl(&mut caller).held_result = if fits || n < 0 { None } else { Some(bytes) };
                    n
                }
                Err(e) => {
                    ctl(&mut caller).held_result = None;
                    e
                }
            }
        },
    )?;
    linker.func_wrap(
        "fw",
        "dev_submit",
        |mut caller: C<'_>, slot: i32, method: i32, args: i32, alen: i32| -> i32 {
            let args = match read_bytes(&mut caller, args, alen) {
                Ok(a) => a,
                Err(e) => return e,
            };
            let c = ctl(&mut caller);
            if c.devices.get(slot as usize).and_then(Option::as_ref).is_none() {
                return E_NODEV;
            }
            if c.async_queue.len() + c.async_results.len() >= ASYNC_MAX {
                return E_AGAIN;
            }
            let ticket = c.next_ticket;
            c.next_ticket = c.next_ticket.wrapping_add(1) & 0x7fff_ffff;
            c.async_queue.push(AsyncReq { ticket, slot, method, args });
            ticket
        },
    )?;
    linker.func_wrap("fw", "dev_result", |mut caller: C<'_>, ticket: i32, ret: i32, rcap: i32| -> i32 {
        if ticket == HELD_RESULT {
            let held = match ctl(&mut caller).held_result.clone() {
                Some(b) => b,
                None => return E_BADHANDLE,
            };
            let n = write_out(&mut caller, ret, rcap, &held);
            if n >= 0 && (n as usize) <= rcap.max(0) as usize {
                ctl(&mut caller).held_result = None;
            }
            return n;
        }
        let result = match ctl(&mut caller).async_results.get(&ticket) {
            Some(r) => r.clone(),
            None => return E_BADHANDLE,
        };
        match result {
            Ok(bytes) => {
                let n = write_out(&mut caller, ret, rcap, &bytes);
                if n >= 0 && (n as usize) <= rcap.max(0) as usize {
                    ctl(&mut caller).async_results.remove(&ticket);
                }
                n
            }
            Err(e) => {
                ctl(&mut caller).async_results.remove(&ticket);
                e
            }
        }
    })?;
    linker.func_wrap("fw", "poll_event", |mut caller: C<'_>, out: i32, cap: i32| -> i32 {
        let c = ctl(&mut caller);
        if c.dropped_events > 0 && c.events.len() < EVENT_QUEUE_MAX {
            let n = c.dropped_events;
            c.dropped_events = 0;
            c.post_event(EV_OVERFLOW, 0, n, &[]);
            // OVERFLOW goes first so the program knows to resync.
            let e = c.events.pop_back().unwrap();
            c.events.push_front(e);
        }
        let ev = match c.events.front() {
            Some(e) => e.clone(),
            None => return 0,
        };
        let n = write_out(&mut caller, out, cap, &ev);
        if n > 0 && (n as usize) <= cap.max(0) as usize {
            ctl(&mut caller).events.pop_front();
        }
        n
    })?;
    Ok(())
}

fn link_fw_image(linker: &mut Linker<Ctx>) -> wasmtime::Result<()> {
    linker.func_wrap("fw", "image_load", |mut caller: C<'_>, ptr: i32, len: i32| -> i32 {
        let raw = match read_bytes(&mut caller, ptr, len) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let c = ctl(&mut caller);
        let engine = c.engine.clone();
        match engine.load_image(&raw, &c.cpu) {
            Ok(img) => {
                let slot = c.images.iter().position(Option::is_none);
                match slot {
                    Some(i) => {
                        c.images[i] = Some(img);
                        i as i32
                    }
                    None => {
                        c.images.push(Some(img));
                        (c.images.len() - 1) as i32
                    }
                }
            }
            Err(e) => e,
        }
    })?;
    linker.func_wrap("fw", "image_free", |mut caller: C<'_>, image: i32| -> i32 {
        match ctl(&mut caller).images.get_mut(image as usize) {
            Some(slot @ Some(_)) => {
                *slot = None;
                0
            }
            _ => E_BADHANDLE,
        }
    })?;
    linker.func_wrap("fw", "exec", |mut caller: C<'_>, image: i32, args: i32, alen: i32| -> i32 {
        if alen < 0 || alen as usize > EXEC_ARGS_MAX {
            return E_INVAL;
        }
        let args = match read_bytes(&mut caller, args, alen) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let c = ctl(&mut caller);
        let img = match c.images.get(image as usize).and_then(Option::clone) {
            Some(i) => i,
            None => return E_BADHANDLE,
        };
        c.exit = Some(Exit::Exec { image: img, args });
        request_exit(&mut caller);
        0
    })?;
    linker.func_wrap("fw", "boot_args", |mut caller: C<'_>, out: i32, cap: i32| -> i32 {
        let args = ctl(&mut caller).boot_args.clone();
        write_out(&mut caller, out, cap, &args)
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------

fn trap_kind(e: &wasmtime::Error) -> i32 {
    use wasmtime::Trap;
    match e.downcast_ref::<Trap>() {
        Some(Trap::UnreachableCodeReached) => 1,
        Some(Trap::MemoryOutOfBounds) => 2,
        Some(Trap::IntegerDivisionByZero) => 3,
        Some(Trap::IntegerOverflow) => 4,
        Some(Trap::StackOverflow) => 5,
        Some(Trap::IndirectCallToNull) => 6,
        Some(Trap::BadSignature) => 7,
        Some(Trap::TableOutOfBounds) => 8,
        Some(Trap::BadConversionToInteger) => 9,
        _ => 0,
    }
}

/// Create a child instance and run it up to its first poll, so it is
/// checkpointable from the start.
pub(crate) fn instantiate_child(
    c: &KernelCtl,
    image: &crate::Image,
    mem_limit: i64,
) -> wasmtime::Result<WeaveInstance> {
    let mut inst = WeaveInstance::new_fresh(
        &c.engine.engine,
        &image.module,
        vec![],
        vec![Box::new(ChildCtl::default())],
        child_linker(),
    )?;
    // Fails here (and co_create reports E_NOMEM) if the program's initial
    // memory plus Weave's shadow stack already exceeds the limit.
    let size = inst.mem_view().map(|v| v.size(0)).unwrap_or(0);
    if size as i64 > mem_limit {
        wasmtime::error::bail!("initial memory {size} exceeds the child's limit {mem_limit}");
    }
    inst.set_memory_limit(Some(mem_limit as usize));
    inst.set_global_i32(names::G_CTR, 0)?;
    inst.set_poller(Poller::UnwindNext);
    inst.call_entry("_start", &[])?;
    Ok(inst)
}

/// The kernel may use whatever memory its children have not reserved.
fn update_kernel_limit(caller: &mut C<'_>) {
    let c = ctl(caller);
    let free = (c.cpu.max_memory as i64 - c.memory_in_use()).max(0) as usize;
    caller.data_mut().set_memory_limit(Some(free));
}

fn child_mut<'a>(c: &'a mut KernelCtl, h: i32) -> Option<&'a mut Child> {
    c.children.get_mut(h as usize).and_then(Option::as_mut)
}

fn link_co(linker: &mut Linker<Ctx>) -> wasmtime::Result<()> {
    linker.func_wrap("co", "co_create", |mut caller: C<'_>, image: i32, mem_limit: i64| -> i32 {
        let c = ctl(&mut caller);
        let img = match c.images.get(image as usize).and_then(Option::clone) {
            Some(i) => i,
            None => return E_BADHANDLE,
        };
        if mem_limit <= 0 {
            return E_INVAL;
        }
        let kernel_size = memory(&mut caller).map(|m| m.data_size(&caller)).unwrap_or(0) as i64;
        let c = ctl(&mut caller);
        let max = c.cpu.max_memory as i64;
        if kernel_size + c.memory_in_use() + mem_limit > max {
            return E_NOMEM;
        }
        let inst = match instantiate_child(c, &img, mem_limit) {
            Ok(i) => i,
            Err(_) => return E_NOMEM,
        };
        let child = Child {
            image: img,
            inst: Some(inst),
            status: STATUS_SUSPENDED,
            mem_limit,
        };
        let h = match c.children.iter().position(Option::is_none) {
            Some(i) => {
                c.children[i] = Some(child);
                i as i32
            }
            None => {
                c.children.push(Some(child));
                (c.children.len() - 1) as i32
            }
        };
        update_kernel_limit(&mut caller);
        h
    })?;

    linker.func_wrap(
        "co",
        "co_resume",
        |mut caller: C<'_>, h: i32, budget: i64, retval: i64, info: i32| -> i32 {
            // The child's budget comes out of the kernel's remaining budget.
            let remaining = match &caller.data().poller {
                Poller::UnwindAfter(n) => *n,
                _ => 0,
            };
            let given = (budget.max(0) as u64).min(remaining);

            let mut record = [0i64; 6]; // reason/code packed below
            let mut args = [0i64; 4];
            let mut used = 0u64;
            let reason;
            {
                let c = ctl(&mut caller);
                let child = match child_mut(c, h) {
                    Some(ch) => ch,
                    None => return E_BADHANDLE,
                };
                if child.status != STATUS_SUSPENDED {
                    reason = if child.status == STATUS_DONE { CO_DONE } else { CO_DEAD };
                    record[0] = reason as i64;
                } else {
                    let inst = child.inst.as_mut().expect("suspended child without instance");
                    {
                        let cc = inst.ctx_mut().service_state::<ChildCtl>(0);
                        if cc.pending_call.take().is_some() {
                            cc.reply = Some(retval);
                        }
                    }
                    inst.set_poller(Poller::UnwindAfter(given));
                    let outcome = inst.resume();
                    let left = match inst.take_poller() {
                        Poller::UnwindAfter(n) => n,
                        _ => 0,
                    };
                    used = given.saturating_sub(left);
                    match outcome {
                        Ok(WorkResult::Unwound) => {
                            let cc = inst.ctx_mut().service_state::<ChildCtl>(0);
                            match cc.pending_call {
                                Some((no, a)) => {
                                    reason = CO_CALL;
                                    record[1] = no as i64;
                                    args = a;
                                }
                                None => reason = CO_YIELD,
                            }
                        }
                        Ok(WorkResult::Done(_)) => {
                            reason = CO_DONE;
                            child.status = STATUS_DONE;
                            child.inst = None;
                        }
                        Err(e) => {
                            reason = CO_DEAD;
                            record[1] = trap_kind(&e) as i64;
                            child.status = STATUS_DEAD;
                            child.inst = None;
                        }
                    }
                    record[0] = reason as i64;
                }
            }
            caller.data_mut().poller = Poller::UnwindAfter(remaining.saturating_sub(used));

            let mut b = Vec::with_capacity(48);
            b.extend_from_slice(&(record[0] as i32).to_le_bytes());
            b.extend_from_slice(&(record[1] as i32).to_le_bytes());
            for a in args {
                b.extend_from_slice(&a.to_le_bytes());
            }
            b.extend_from_slice(&(used as i64).to_le_bytes());
            if let Err(e) = write_bytes(&mut caller, info, &b) {
                return e;
            }
            reason
        },
    )?;

    linker.func_wrap("co", "co_status", |mut caller: C<'_>, h: i32| -> i32 {
        match child_mut(ctl(&mut caller), h) {
            Some(ch) => ch.status,
            None => E_BADHANDLE,
        }
    })?;

    linker.func_wrap("co", "co_peek", |mut caller: C<'_>, h: i32, addr: i32, buf: i32, len: i32| -> i32 {
        if len < 0 {
            return E_INVAL;
        }
        let bytes = {
            let c = ctl(&mut caller);
            let inst = match child_mut(c, h).and_then(|ch| ch.inst.as_mut()) {
                Some(i) => i,
                None => return E_BADHANDLE,
            };
            let view = match inst.mem_view() {
                Ok(v) => v,
                Err(_) => return E_FAULT,
            };
            let start = addr as u32 as usize;
            let end = match start.checked_add(len as usize) {
                Some(e) if e <= view.size(0) => e,
                _ => return E_FAULT,
            };
            let mut out = vec![0u8; end - start];
            view.read(0, start, &mut out);
            out
        };
        err_code(write_bytes(&mut caller, buf, &bytes).map(|()| len))
    })?;

    linker.func_wrap("co", "co_poke", |mut caller: C<'_>, h: i32, addr: i32, buf: i32, len: i32| -> i32 {
        let bytes = match read_bytes(&mut caller, buf, len) {
            Ok(b) => b,
            Err(e) => return e,
        };
        let c = ctl(&mut caller);
        let inst = match child_mut(c, h).and_then(|ch| ch.inst.as_mut()) {
            Some(i) => i,
            None => return E_BADHANDLE,
        };
        let size = match inst.mem_view() {
            Ok(v) => v.size(0),
            Err(_) => return E_FAULT,
        };
        let start = addr as u32 as usize;
        match start.checked_add(bytes.len()) {
            Some(end) if end <= size => {}
            _ => return E_FAULT,
        }
        match inst.write_mem(0, start, &bytes) {
            Ok(()) => len,
            Err(_) => E_FAULT,
        }
    })?;

    linker.func_wrap("co", "co_close", |mut caller: C<'_>, h: i32| -> i32 {
        let r = match ctl(&mut caller).children.get_mut(h as usize) {
            Some(slot @ Some(_)) => {
                *slot = None;
                0
            }
            _ => E_BADHANDLE,
        };
        update_kernel_limit(&mut caller);
        r
    })?;
    Ok(())
}
