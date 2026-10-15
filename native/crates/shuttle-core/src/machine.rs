//! One computer: the top-level guest (BIOS, then OS), its children, devices,
//! events and the per-tick scheduler.

use crate::abi::*;
use crate::imports;
use crate::{CpuSpec, Device, Image, ShuttleEngine};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use wasmtime::Result;
use weave_core::names;
use weave_wasmtime::{InstanceState, Poller, WeaveInstance, WorkResult};

/// What a stopped guest asked the firmware to do.
pub(crate) enum Exit {
    Hlt(i64),
    Yield,
    Exec { image: Arc<Image>, args: Vec<u8> },
    Shutdown,
    Reboot,
}

/// Per-child host state, stored in the child's own Store.
#[derive(Default)]
pub(crate) struct ChildCtl {
    /// Set by the first `sys.call`; reported by the kernel's `co_resume`.
    pub pending_call: Option<(i32, [i64; 4])>,
    /// Result for the re-executed `sys.call` after resume.
    pub reply: Option<i64>,
}

pub(crate) struct Child {
    pub image: Arc<Image>,
    pub inst: Option<WeaveInstance>,
    pub status: i32,
    pub mem_limit: i64,
}

pub(crate) struct AsyncReq {
    pub ticket: i32,
    pub slot: i32,
    pub method: i32,
    pub args: Vec<u8>,
}

/// Machine-wide host state. While a top-level guest exists it lives inside
/// that guest's Store (so host functions can reach it); otherwise the
/// [`Machine`] keeps it.
pub(crate) struct KernelCtl {
    pub engine: Arc<ShuttleEngine>,
    pub cpu: CpuSpec,
    pub devices: Vec<Option<Box<dyn Device>>>,
    pub images: Vec<Option<Arc<Image>>>,
    pub children: Vec<Option<Child>>,
    pub events: VecDeque<Vec<u8>>,
    pub dropped_events: u32,
    pub tick: u64,
    pub logs: Vec<LogLine>,
    pub log_lines_this_tick: usize,
    pub exit: Option<Exit>,
    /// Result for the re-executed exit call (hlt, yield) after resume.
    pub exit_reply: Option<i64>,
    /// A `dev_call` result that did not fit; fetched with `dev_result(-1)`.
    pub held_result: Option<Vec<u8>>,
    pub boot_args: Vec<u8>,
    pub next_ticket: i32,
    pub async_queue: Vec<AsyncReq>,
    pub async_results: BTreeMap<i32, Result<Vec<u8>, i32>>,
}

impl KernelCtl {
    pub fn post_event(&mut self, kind: u16, slot: u16, param: u32, payload: &[u8]) {
        if self.events.len() >= EVENT_QUEUE_MAX {
            self.dropped_events += 1;
            return;
        }
        let mut e = Vec::with_capacity(20 + payload.len());
        e.extend_from_slice(&kind.to_le_bytes());
        e.extend_from_slice(&slot.to_le_bytes());
        e.extend_from_slice(&param.to_le_bytes());
        e.extend_from_slice(&(self.tick as i64).to_le_bytes());
        e.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        e.extend_from_slice(payload);
        self.events.push_back(e);
    }

    pub fn log(&mut self, text: String) {
        if self.log_lines_this_tick >= LOG_LINES_PER_TICK {
            return;
        }
        self.log_lines_this_tick += 1;
        self.logs.push(LogLine {
            tick: self.tick,
            text,
        });
    }

    pub fn memory_in_use(&self) -> i64 {
        self.children
            .iter()
            .flatten()
            .filter(|c| c.status == STATUS_SUSPENDED)
            .map(|c| c.mem_limit)
            .sum()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    pub tick: u64,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MachineState {
    /// Runs every tick.
    Running,
    /// Stopped in `hlt` until an event arrives or the given tick (0 = never).
    Halted { until: u64 },
    Off,
    Crashed(String),
}

pub struct Machine {
    pub(crate) eeprom: Arc<Image>,
    pub(crate) kernel: Option<WeaveInstance>,
    pub(crate) kernel_image: Option<Arc<Image>>,
    /// Holds the ctl while no top-level instance exists.
    pub(crate) spare_ctl: Option<Box<KernelCtl>>,
    pub(crate) state: MachineState,
}

impl Machine {
    /// Build a powered-off computer. `eeprom` is the BIOS program.
    pub fn new(
        engine: &Arc<ShuttleEngine>,
        cpu: CpuSpec,
        eeprom: &[u8],
        devices: Vec<Box<dyn Device>>,
    ) -> std::result::Result<Machine, i32> {
        let eeprom = engine.load_image(eeprom, &cpu)?;
        let ctl = KernelCtl {
            engine: engine.clone(),
            cpu,
            devices: devices.into_iter().map(Some).collect(),
            images: Vec::new(),
            children: Vec::new(),
            events: VecDeque::new(),
            dropped_events: 0,
            tick: 0,
            logs: Vec::new(),
            log_lines_this_tick: 0,
            exit: None,
            exit_reply: None,
            held_result: None,
            boot_args: Vec::new(),
            next_ticket: 0,
            async_queue: Vec::new(),
            async_results: BTreeMap::new(),
        };
        Ok(Machine {
            eeprom,
            kernel: None,
            kernel_image: None,
            spare_ctl: Some(Box::new(ctl)),
            state: MachineState::Off,
        })
    }

    pub fn state(&self) -> &MachineState {
        &self.state
    }

    pub(crate) fn ctl(&mut self) -> &mut KernelCtl {
        match self.kernel.as_mut() {
            Some(k) => kernel_ctl_slot(k).as_mut().expect("kernel lost its ctl"),
            None => self.spare_ctl.as_mut().expect("machine lost its ctl"),
        }
    }

    pub fn tick_count(&mut self) -> u64 {
        self.ctl().tick
    }

    /// Lines written with `fw.log` since the last call.
    pub fn drain_log(&mut self) -> Vec<LogLine> {
        std::mem::take(&mut self.ctl().logs)
    }

    /// Test hook for control experiments: forget the host-side child table,
    /// as a broken restore would. Never used by real embedders.
    #[doc(hidden)]
    pub fn debug_drop_children(&mut self) {
        self.ctl().children.clear();
    }

    /// Queue an event as a device would (key press, redstone change, ...).
    pub fn post_event(&mut self, kind: u16, slot: u16, param: u32, payload: &[u8]) {
        self.ctl().post_event(kind, slot, param, payload);
    }

    /// Turn the computer on: the firmware starts the BIOS from the EEPROM.
    pub fn power_on(&mut self) -> Result<()> {
        if !matches!(self.state, MachineState::Off | MachineState::Crashed(_)) {
            return Ok(());
        }
        let bios = self.eeprom.clone();
        self.start_top_level(bios, Vec::new())
    }

    /// Detach the ctl from the current top-level instance (dropping it).
    fn take_ctl(&mut self) -> Box<KernelCtl> {
        match self.kernel.take() {
            Some(mut k) => kernel_ctl_slot(&mut k).take().expect("kernel lost its ctl"),
            None => self.spare_ctl.take().expect("machine lost its ctl"),
        }
    }

    /// Replace the top-level program (power on, `exec`, reboot). Memory and
    /// all children are discarded; images stay loaded.
    fn start_top_level(&mut self, image: Arc<Image>, boot_args: Vec<u8>) -> Result<()> {
        let mut ctl = self.take_ctl();
        ctl.children.clear();
        ctl.exit = None;
        ctl.exit_reply = None;
        ctl.held_result = None;
        ctl.boot_args = boot_args;
        let max_memory = ctl.cpu.max_memory as usize;
        let engine = ctl.engine.engine.clone();
        let mut inst = match WeaveInstance::new_fresh(
            &engine,
            &image.module,
            vec![],
            vec![Box::new(Some(ctl))],
            imports::kernel_linker(),
        ) {
            Ok(i) => i,
            Err(e) => {
                // The ctl went down with the failed instantiation; rebuild it
                // is not possible here, so this is fatal for the prototype.
                return Err(e.context("instantiating top-level program"));
            }
        };
        inst.set_memory_limit(Some(max_memory));
        // Run up to the first poll so the instance is checkpointable at once.
        inst.set_global_i32(names::G_CTR, 0)?;
        inst.set_poller(Poller::UnwindNext);
        match inst.call_entry("_start", &[])? {
            WorkResult::Unwound => {}
            WorkResult::Done(_) => {}
        }
        kernel_ctl_slot(&mut inst).as_mut().unwrap().exit = None;
        self.kernel = Some(inst);
        self.kernel_image = Some(image);
        self.state = MachineState::Running;
        Ok(())
    }

    fn power_off(&mut self, state: MachineState) {
        let mut ctl = self.take_ctl();
        ctl.children.clear();
        ctl.exit = None;
        self.spare_ctl = Some(ctl);
        self.kernel_image = None;
        self.state = state;
    }

    /// Advance one tick.
    pub fn tick(&mut self) -> Result<&MachineState> {
        {
            let ctl = self.ctl();
            ctl.tick += 1;
            ctl.log_lines_this_tick = 0;
        }
        self.run_async_requests();

        match self.state.clone() {
            MachineState::Off | MachineState::Crashed(_) => return Ok(&self.state),
            MachineState::Halted { until } => {
                let tick = self.ctl().tick;
                let has_event = !self.ctl().events.is_empty();
                if has_event {
                    self.ctl().exit_reply = Some(1);
                } else if until != 0 && tick >= until {
                    self.ctl().exit_reply = Some(0);
                } else {
                    return Ok(&self.state);
                }
                self.state = MachineState::Running;
            }
            MachineState::Running => {}
        }

        let clock = self.ctl().cpu.clock;
        let kernel = self.kernel.as_mut().expect("running machine without a kernel");
        kernel.set_poller(Poller::UnwindAfter(clock));
        let outcome = if kernel.state() == InstanceState::Paused {
            kernel.resume()
        } else {
            kernel.call_entry("_start", &[])
        };

        match outcome {
            Err(e) => {
                let msg = format!("{e:#}");
                self.ctl().log(format!("firmware: kernel panic: {msg}"));
                self.power_off(MachineState::Crashed(msg));
            }
            Ok(WorkResult::Done(_)) => {
                self.ctl().log("firmware: top-level program returned".into());
                self.power_off(MachineState::Off);
            }
            Ok(WorkResult::Unwound) => {
                let exit = self.ctl().exit.take();
                match exit {
                    None => {}
                    Some(Exit::Yield) => self.ctl().exit_reply = Some(0),
                    Some(Exit::Hlt(timeout)) => {
                        let tick = self.ctl().tick;
                        let until = if timeout <= 0 { 0 } else { tick + timeout as u64 };
                        self.state = MachineState::Halted { until };
                    }
                    Some(Exit::Exec { image, args }) => {
                        self.start_top_level(image, args)?;
                    }
                    Some(Exit::Shutdown) => {
                        self.ctl().log("firmware: power off".into());
                        self.power_off(MachineState::Off);
                    }
                    Some(Exit::Reboot) => {
                        self.ctl().log("firmware: reboot".into());
                        let bios = self.eeprom.clone();
                        self.start_top_level(bios, Vec::new())?;
                    }
                }
            }
        }
        Ok(&self.state)
    }

    /// Complete queued `dev_submit` calls. In the mod this happens on the
    /// server thread; here it simply runs at the start of the next tick.
    fn run_async_requests(&mut self) {
        let ctl = self.ctl();
        let queue = std::mem::take(&mut ctl.async_queue);
        for req in queue {
            let result = match ctl.devices.get_mut(req.slot as usize).and_then(Option::as_mut) {
                Some(dev) => dev.call(req.method, &req.args),
                None => Err(E_NODEV),
            };
            let status = match &result {
                Ok(bytes) => bytes.len() as i32,
                Err(code) => *code,
            };
            ctl.async_results.insert(req.ticket, result);
            ctl.post_event(EV_COMPLETE, req.slot as u16, req.ticket as u32, &status.to_le_bytes());
        }
    }
}

/// The ctl slot inside a top-level instance's Store.
pub(crate) fn kernel_ctl_slot(k: &mut WeaveInstance) -> &mut Option<Box<KernelCtl>> {
    k.ctx_mut().service_state::<Option<Box<KernelCtl>>>(0)
}
