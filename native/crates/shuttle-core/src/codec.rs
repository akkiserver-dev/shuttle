//! Machine snapshot format (gzip-compressed, versioned).
//!
//! Layout (all integers little-endian), before compression:
//!   magic "SHTL", u16 format
//!   cpu: vendor, model, arch, features (str), clock u64, max_memory u64
//!   tick u64, state u8 (+ u64 until / str message),
//!   exit_reply (opt i64), held_result (opt bytes)
//!   boot_args bytes, dropped_events u32, events [bytes]
//!   next_ticket i32, async results [(ticket i32, ok u8, bytes|code)],
//!   async queue [(ticket, slot, method, args)]
//!   image store [(hash, raw bytes)]       -- every image referenced below
//!   eeprom hash, kernel image (opt hash), kernel snapshot (opt bytes)
//!   image table [opt hash]
//!   children [opt (hash, status i32, mem_limit i64, pending call, reply, snapshot)]
//!
//! Device state is not included: the embedder saves it (Device.save on the
//! Java side). Queued `dev_submit` requests are saved and run after restore.

use crate::imports;
use crate::machine::{
    AsyncReq, ChildCtl, Child, KernelCtl, Machine, MachineState, kernel_ctl_slot,
};
use crate::{CpuSpec, Device, Image, ShuttleEngine};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::sync::Arc;
use wasmtime::error::{bail, format_err, Result};
use weave_core::snapshot::Snapshot;
use weave_wasmtime::WeaveInstance;

const MAGIC: &[u8; 4] = b"SHTL";
const FORMAT: u16 = 2;

struct W(Vec<u8>);
impl W {
    fn u8(&mut self, v: u8) { self.0.push(v); }
    fn u16(&mut self, v: u16) { self.0.extend_from_slice(&v.to_le_bytes()); }
    fn u32(&mut self, v: u32) { self.0.extend_from_slice(&v.to_le_bytes()); }
    fn u64(&mut self, v: u64) { self.0.extend_from_slice(&v.to_le_bytes()); }
    fn i32(&mut self, v: i32) { self.0.extend_from_slice(&v.to_le_bytes()); }
    fn i64(&mut self, v: i64) { self.0.extend_from_slice(&v.to_le_bytes()); }
    fn bytes(&mut self, b: &[u8]) { self.u32(b.len() as u32); self.0.extend_from_slice(b); }
    fn str(&mut self, s: &str) { self.bytes(s.as_bytes()); }
    fn hash(&mut self, h: &[u8; 32]) { self.0.extend_from_slice(h); }
}

struct R<'a>(&'a [u8]);
impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.0.len() < n {
            bail!("snapshot truncated");
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8> { Ok(self.take(1)?[0]) }
    fn u16(&mut self) -> Result<u16> { Ok(u16::from_le_bytes(self.take(2)?.try_into()?)) }
    fn u32(&mut self) -> Result<u32> { Ok(u32::from_le_bytes(self.take(4)?.try_into()?)) }
    fn u64(&mut self) -> Result<u64> { Ok(u64::from_le_bytes(self.take(8)?.try_into()?)) }
    fn i32(&mut self) -> Result<i32> { Ok(i32::from_le_bytes(self.take(4)?.try_into()?)) }
    fn i64(&mut self) -> Result<i64> { Ok(i64::from_le_bytes(self.take(8)?.try_into()?)) }
    fn bytes(&mut self) -> Result<Vec<u8>> { let n = self.u32()? as usize; Ok(self.take(n)?.to_vec()) }
    fn str(&mut self) -> Result<String> { Ok(String::from_utf8(self.bytes()?)?) }
    fn hash(&mut self) -> Result<[u8; 32]> { Ok(self.take(32)?.try_into()?) }
}

impl Machine {
    /// Capture the whole computer at a tick boundary as gzip bytes
    /// (fits an NBT ByteArrayTag).
    pub fn capture(&mut self) -> Result<Vec<u8>> {
        let eeprom = self.eeprom.clone();
        let kernel_image = self.kernel_image.clone();
        let state = self.state.clone();
        let kernel_snap = match self.kernel.as_mut() {
            Some(k) => Some(k.checkpoint()?.encode()),
            None => None,
        };
        let ctl = self.ctl();

        // Every image the machine refers to, deduplicated by hash.
        let mut store: BTreeMap<[u8; 32], Arc<Image>> = BTreeMap::new();
        store.insert(eeprom.hash, eeprom.clone());
        if let Some(k) = &kernel_image {
            store.insert(k.hash, k.clone());
        }
        for img in ctl.images.iter().flatten() {
            store.insert(img.hash, img.clone());
        }
        for ch in ctl.children.iter().flatten() {
            store.insert(ch.image.hash, ch.image.clone());
        }

        let mut w = W(Vec::new());
        w.0.extend_from_slice(MAGIC);
        w.u16(FORMAT);
        w.str(&ctl.cpu.vendor);
        w.str(&ctl.cpu.model);
        w.str(&ctl.cpu.arch);
        w.str(&ctl.cpu.features);
        w.u64(ctl.cpu.clock);
        w.u64(ctl.cpu.max_memory);
        w.u64(ctl.tick);
        match &state {
            MachineState::Running => w.u8(0),
            MachineState::Halted { until } => { w.u8(1); w.u64(*until); }
            MachineState::Off => w.u8(2),
            MachineState::Crashed(m) => { w.u8(3); w.str(m); }
        }
        match ctl.exit_reply {
            Some(v) => { w.u8(1); w.i64(v); }
            None => w.u8(0),
        }
        match &ctl.held_result {
            Some(b) => { w.u8(1); w.bytes(b); }
            None => w.u8(0),
        }
        w.bytes(&ctl.boot_args);
        w.u32(ctl.dropped_events);
        w.u32(ctl.events.len() as u32);
        for e in &ctl.events {
            w.bytes(e);
        }
        w.i32(ctl.next_ticket);
        w.u32(ctl.async_results.len() as u32);
        for (t, r) in &ctl.async_results {
            w.i32(*t);
            match r {
                Ok(b) => { w.u8(1); w.bytes(b); }
                Err(c) => { w.u8(0); w.i32(*c); }
            }
        }
        w.u32(ctl.async_queue.len() as u32);
        for q in &ctl.async_queue {
            w.i32(q.ticket);
            w.i32(q.slot);
            w.i32(q.method);
            w.bytes(&q.args);
        }
        w.u32(store.len() as u32);
        for (h, img) in &store {
            w.hash(h);
            w.bytes(&img.raw);
        }
        w.hash(&eeprom.hash);
        match (&kernel_image, &kernel_snap) {
            (Some(img), Some(snap)) => { w.u8(1); w.hash(&img.hash); w.bytes(snap); }
            _ => w.u8(0),
        }
        w.u32(ctl.images.len() as u32);
        for img in &ctl.images {
            match img {
                Some(i) => { w.u8(1); w.hash(&i.hash); }
                None => w.u8(0),
            }
        }
        let n_children = ctl.children.len();
        w.u32(n_children as u32);
        for i in 0..n_children {
            let ctl = self.ctl();
            let ch = match ctl.children[i].as_mut() {
                Some(ch) => ch,
                None => { w.u8(0); continue; }
            };
            w.u8(1);
            w.hash(&ch.image.hash);
            w.i32(ch.status);
            w.i64(ch.mem_limit);
            match ch.inst.as_mut() {
                Some(inst) => {
                    let (call, reply) = {
                        let cc = inst.ctx_mut().service_state::<ChildCtl>(0);
                        (cc.pending_call, cc.reply)
                    };
                    match call {
                        Some((no, a)) => {
                            w.u8(1);
                            w.i32(no);
                            for v in a { w.i64(v); }
                        }
                        None => w.u8(0),
                    }
                    match reply {
                        Some(v) => { w.u8(1); w.i64(v); }
                        None => w.u8(0),
                    }
                    w.u8(1);
                    w.bytes(&inst.checkpoint()?.encode());
                }
                None => {
                    w.u8(0); // no pending call
                    w.u8(0); // no reply
                    w.u8(0); // no instance
                }
            }
        }

        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&w.0)?;
        Ok(gz.finish()?)
    }

    /// Rebuild a computer from [`Machine::capture`] output. Devices are
    /// supplied again by the embedder, in the same slot order.
    pub fn restore(
        engine: &Arc<ShuttleEngine>,
        bytes: &[u8],
        devices: Vec<Box<dyn Device>>,
    ) -> Result<Machine> {
        let mut raw = Vec::new();
        GzDecoder::new(bytes).read_to_end(&mut raw)?;
        let mut r = R(&raw);
        if r.take(4)? != MAGIC {
            bail!("not a Shuttle snapshot");
        }
        let format = r.u16()?;
        if format != FORMAT {
            bail!("unsupported snapshot format {format}");
        }
        let cpu = CpuSpec {
            vendor: r.str()?,
            model: r.str()?,
            arch: r.str()?,
            features: r.str()?,
            clock: r.u64()?,
            max_memory: r.u64()?,
        };
        let tick = r.u64()?;
        let state = match r.u8()? {
            0 => MachineState::Running,
            1 => MachineState::Halted { until: r.u64()? },
            2 => MachineState::Off,
            3 => MachineState::Crashed(r.str()?),
            s => bail!("bad machine state {s}"),
        };
        let exit_reply = if r.u8()? == 1 { Some(r.i64()?) } else { None };
        let held_result = if r.u8()? == 1 { Some(r.bytes()?) } else { None };
        let boot_args = r.bytes()?;
        let dropped_events = r.u32()?;
        let mut events = VecDeque::new();
        for _ in 0..r.u32()? {
            events.push_back(r.bytes()?);
        }
        let next_ticket = r.i32()?;
        let mut async_results = BTreeMap::new();
        for _ in 0..r.u32()? {
            let t = r.i32()?;
            let v = if r.u8()? == 1 { Ok(r.bytes()?) } else { Err(r.i32()?) };
            async_results.insert(t, v);
        }
        let mut async_queue = Vec::new();
        for _ in 0..r.u32()? {
            async_queue.push(AsyncReq {
                ticket: r.i32()?,
                slot: r.i32()?,
                method: r.i32()?,
                args: r.bytes()?,
            });
        }
        let mut store: BTreeMap<[u8; 32], Arc<Image>> = BTreeMap::new();
        for _ in 0..r.u32()? {
            let h = r.hash()?;
            let bytes = r.bytes()?;
            let img = engine
                .load_unchecked(&bytes)
                .map_err(|c| format_err!("image failed to load on restore ({c})"))?;
            if img.hash != h {
                bail!("image hash mismatch in snapshot");
            }
            store.insert(h, img);
        }
        let get = |h: &[u8; 32]| store.get(h).cloned().ok_or_else(|| format_err!("snapshot refers to a missing image"));
        let eeprom = get(&r.hash()?)?;
        let kernel_part = if r.u8()? == 1 {
            Some((get(&r.hash()?)?, r.bytes()?))
        } else {
            None
        };
        let mut images = Vec::new();
        for _ in 0..r.u32()? {
            images.push(if r.u8()? == 1 { Some(get(&r.hash()?)?) } else { None });
        }

        let mut ctl = Box::new(KernelCtl {
            engine: engine.clone(),
            cpu,
            devices: devices.into_iter().map(Some).collect(),
            images,
            children: Vec::new(),
            events,
            dropped_events,
            tick,
            logs: Vec::new(),
            log_lines_this_tick: 0,
            exit: None,
            exit_reply,
            held_result,
            boot_args,
            next_ticket,
            async_queue,
            async_results,
        });

        for _ in 0..r.u32()? {
            if r.u8()? == 0 {
                ctl.children.push(None);
                continue;
            }
            let image = get(&r.hash()?)?;
            let status = r.i32()?;
            let mem_limit = r.i64()?;
            let pending_call = if r.u8()? == 1 {
                let no = r.i32()?;
                let mut a = [0i64; 4];
                for v in &mut a {
                    *v = r.i64()?;
                }
                Some((no, a))
            } else {
                None
            };
            let reply = if r.u8()? == 1 { Some(r.i64()?) } else { None };
            let inst = if r.u8()? == 1 {
                let snap = Snapshot::decode(&r.bytes()?)?;
                let mut inst = WeaveInstance::new_restored(
                    &engine.engine,
                    &image.module,
                    vec![],
                    vec![Box::new(ChildCtl { pending_call, reply })],
                    imports::child_linker(),
                )?;
                inst.restore(&snap)?;
                inst.set_memory_limit(Some(mem_limit as usize));
                Some(inst)
            } else {
                None
            };
            ctl.children.push(Some(Child { image, inst, status, mem_limit }));
        }
        if !r.0.is_empty() {
            bail!("trailing bytes in snapshot");
        }

        let (kernel, kernel_image, spare_ctl) = match kernel_part {
            Some((image, snap)) => {
                let snap = Snapshot::decode(&snap)?;
                let mut inst = WeaveInstance::new_restored(
                    &engine.engine,
                    &image.module,
                    vec![],
                    vec![Box::new(Some(ctl))],
                    imports::kernel_linker(),
                )?;
                inst.restore(&snap)?;
                let ctl = kernel_ctl_slot(&mut inst).as_ref().expect("kernel ctl");
                let free = (ctl.cpu.max_memory as i64 - ctl.memory_in_use()).max(0) as usize;
                inst.set_memory_limit(Some(free));
                (Some(inst), Some(image), None)
            }
            None => (None, None, Some(ctl)),
        };
        Ok(Machine {
            eeprom,
            kernel,
            kernel_image,
            spare_ctl,
            state,
        })
    }
}
