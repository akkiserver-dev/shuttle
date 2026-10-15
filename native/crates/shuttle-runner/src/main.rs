//! Headless Shuttle emulator.
//!
//!   shuttle-runner run    <bios.wasm> <disk0> [disk1 ...]
//!   shuttle-runner verify <bios.wasm> <disk0> [disk1 ...]
//!
//! `run` boots the computer and prints its log until it powers off.
//! `verify` runs the same boot once without interruption, then again with a
//! capture -> restore (into a brand-new machine) at several tick boundaries,
//! and checks that the log streams are identical.

use shuttle_core::{CpuSpec, DevInfo, Device, LogLine, Machine, MachineState, ShuttleEngine};
use std::sync::Arc;
use std::time::Instant;

const MAX_TICKS: u64 = 2000;

/// Read-only disk. Methods: 0 = size() -> u64, 1 = read(off: u64, len: u32).
struct RomDisk {
    data: Arc<Vec<u8>>,
    address: [u8; 16],
}

impl RomDisk {
    fn new(data: Arc<Vec<u8>>, index: u8) -> RomDisk {
        let mut address = [0u8; 16];
        // deterministic pseudo-UUID derived from contents and position
        let mut h: u64 = 0xcbf29ce484222325 ^ index as u64;
        for b in data.iter() {
            h = (h ^ *b as u64).wrapping_mul(0x100000001b3);
        }
        address[..8].copy_from_slice(&h.to_le_bytes());
        address[8..].copy_from_slice(&(h.rotate_left(17) ^ 0x5bd1e995).to_le_bytes());
        RomDisk { data, address }
    }
}

impl Device for RomDisk {
    fn info(&self) -> DevInfo {
        DevInfo { class: 0x03, vendor: 0x0001, device: 0x0001, flags: 0, address: self.address }
    }
    fn call(&mut self, method: i32, args: &[u8]) -> Result<Vec<u8>, i32> {
        match method {
            0 => Ok((self.data.len() as u64).to_le_bytes().to_vec()),
            1 => {
                if args.len() != 12 {
                    return Err(-1);
                }
                let off = u64::from_le_bytes(args[0..8].try_into().unwrap()) as usize;
                let len = u32::from_le_bytes(args[8..12].try_into().unwrap()) as usize;
                let end = off.checked_add(len).ok_or(-1)?;
                self.data.get(off..end).map(|s| s.to_vec()).ok_or(-11)
            }
            _ => Err(-4),
        }
    }
}

fn devices(disks: &[Arc<Vec<u8>>]) -> Vec<Box<dyn Device>> {
    disks
        .iter()
        .enumerate()
        .map(|(i, d)| Box::new(RomDisk::new(d.clone(), i as u8)) as Box<dyn Device>)
        .collect()
}

fn finished(state: &MachineState) -> bool {
    matches!(state, MachineState::Off | MachineState::Crashed(_))
}

/// Boot and run to power-off. If `restore_at` is set, capture after that
/// tick and continue on a restored copy. Returns the log and snapshot size.
fn run(
    engine: &Arc<ShuttleEngine>,
    bios: &[u8],
    disks: &[Arc<Vec<u8>>],
    restore_at: Option<u64>,
    echo: bool,
    drop_children: bool,
) -> (Vec<LogLine>, Option<usize>, u64) {
    let mut m = Machine::new(engine, CpuSpec::default(), bios, devices(disks))
        .unwrap_or_else(|e| panic!("BIOS rejected: {e}"));
    m.power_on().expect("power on");
    let mut log = m.drain_log();
    let mut snap_size = None;
    let mut ticks = 0;
    for t in 1..=MAX_TICKS {
        let state = m.tick().expect("tick").clone();
        ticks = t;
        let lines = m.drain_log();
        if echo {
            for l in &lines {
                println!("[{:>4}] {}", l.tick, l.text);
            }
        }
        log.extend(lines);
        if finished(&state) {
            break;
        }
        if restore_at == Some(t) {
            let bytes = m.capture().expect("capture");
            snap_size = Some(bytes.len());
            drop(m);
            m = Machine::restore(engine, &bytes, devices(disks)).expect("restore");
            if drop_children {
                m.debug_drop_children();
            }
        }
    }
    (log, snap_size, ticks)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 || (args[1] != "run" && args[1] != "verify") {
        eprintln!("usage: shuttle-runner run|verify <bios.wasm> <disk0> [disk1 ...]");
        std::process::exit(2);
    }
    let bios = std::fs::read(&args[2]).expect("read bios");
    let disks: Vec<Arc<Vec<u8>>> = args[3..]
        .iter()
        .map(|p| Arc::new(std::fs::read(p).expect("read disk")))
        .collect();
    let engine = ShuttleEngine::new().expect("engine");

    let t0 = Instant::now();
    let (golden, _, ticks) = run(&engine, &bios, &disks, None, args[1] == "run", false);
    println!(
        "-- uninterrupted run: {} log lines, {} ticks, {:.1} ms (incl. transform/compile)",
        golden.len(),
        ticks,
        t0.elapsed().as_secs_f64() * 1e3
    );
    if args[1] == "run" {
        return;
    }

    let points: Vec<u64> = (1..ticks).collect();
    let mut ok = 0;
    for &p in &points {
        let t = Instant::now();
        let (log, size, _) = run(&engine, &bios, &disks, Some(p), false, false);
        let same = log == golden;
        if same {
            ok += 1;
        }
        println!(
            "restore after tick {:>4}: snapshot {:>6} bytes (gzip), log {} ({:.1} ms)",
            p,
            size.unwrap_or(0),
            if same { "identical" } else { "DIFFERENT" },
            t.elapsed().as_secs_f64() * 1e3
        );
        if !same {
            for (i, (a, b)) in golden.iter().zip(log.iter()).enumerate() {
                if a != b {
                    println!("  first difference at line {i}: {:?} vs {:?}", a, b);
                    break;
                }
            }
            if golden.len() != log.len() {
                println!("  lengths differ: {} vs {}", golden.len(), log.len());
            }
        }
    }
    println!("-- {ok}/{} restore points identical; images cached: {}", points.len(), engine.cached_images());

    // Control: a restore that loses the host-side child table must diverge,
    // otherwise the comparison above would prove nothing.
    let mut caught = 0;
    let control_points: Vec<u64> = points.iter().copied().filter(|&p| p >= 3).collect();
    for &p in &control_points {
        let (log, _, _) = run(&engine, &bios, &disks, Some(p), false, true);
        if log != golden {
            caught += 1;
        }
    }
    println!(
        "-- control (child table dropped on restore): {caught}/{} runs diverged as expected",
        control_points.len()
    );
    if caught != control_points.len() {
        std::process::exit(1);
    }
    if ok != points.len() {
        std::process::exit(1);
    }
}
