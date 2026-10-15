//! shuttle-core: the native engine of the Shuttle runtime library.
//!
//! It runs Wasm guests (BIOS, OS kernel, child programs) on Wasmtime, makes
//! them checkpointable with Weave, and implements the firmware ABI draft 0.1
//! (`fw.*`, `co.*`, `sys.call`). Nothing in this crate knows about Minecraft:
//! devices are plugged in through the [`Device`] trait.

pub mod abi;
mod codec;
mod imports;
mod machine;

pub use machine::{LogLine, Machine, MachineState};

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wasmtime::Engine;
use weave_transform::TransformOptions;
use weave_wasmtime::WeaveModule;

/// The CPU as built in the assembler: names chosen by the player, the rest
/// decided by the parts.
#[derive(Clone, Debug)]
pub struct CpuSpec {
    pub vendor: String,
    pub model: String,
    pub arch: String,
    /// Comma separated Wasm feature names, e.g. `"simd,bulk-memory"`.
    pub features: String,
    /// Budget per tick, counted in Weave polls.
    pub clock: u64,
    /// Total memory for the kernel and all children, in bytes.
    pub max_memory: u64,
}

impl CpuSpec {
    pub fn has_feature(&self, name: &str) -> bool {
        self.features.split(',').any(|f| f.trim() == name)
    }
}

impl Default for CpuSpec {
    fn default() -> Self {
        CpuSpec {
            vendor: "Akki Semiconductor".into(),
            model: "AK-486 Turbo".into(),
            arch: "wasm32".into(),
            features: "bulk-memory".into(),
            clock: 2000,
            max_memory: 64 * 1024 * 1024,
        }
    }
}

/// Basic identity of a device, as returned by `fw.dev_info`.
#[derive(Clone, Copy, Debug)]
pub struct DevInfo {
    pub class: u16,
    pub vendor: u16,
    pub device: u16,
    pub flags: u16,
    pub address: [u8; 16],
}

/// A device plugged into a bus slot. On the Java side this is backed by
/// `Device.invoke`; here it is a plain Rust trait.
pub trait Device: Send {
    fn info(&self) -> DevInfo;
    /// Method table as returned by `fw.dev_describe` (format not fixed yet).
    fn describe(&self) -> Vec<u8> {
        Vec::new()
    }
    /// Run one method. `Ok` carries the result bytes, `Err` an error code.
    fn call(&mut self, method: i32, args: &[u8]) -> Result<Vec<u8>, i32>;
}

/// A loaded program: the original bytes plus its Weave-transformed module.
pub struct Image {
    pub hash: [u8; 32],
    pub raw: Arc<Vec<u8>>,
    pub module: WeaveModule,
}

/// Process-wide engine: one Wasmtime engine and the image cache shared by
/// every machine.
pub struct ShuttleEngine {
    pub(crate) engine: Engine,
    cache: Mutex<HashMap<[u8; 32], Arc<Image>>>,
    transform: TransformOptions,
}

impl ShuttleEngine {
    pub fn new() -> wasmtime::Result<Arc<ShuttleEngine>> {
        Ok(Arc::new(ShuttleEngine {
            engine: weave_wasmtime::default_engine()?,
            cache: Mutex::new(HashMap::new()),
            transform: TransformOptions {
                poll_period: 64,
                stack_pages: 4,
                yielding_imports: abi::YIELDING_IMPORTS
                    .iter()
                    .map(|(m, n)| (m.to_string(), n.to_string()))
                    .collect(),
            },
        }))
    }

    /// Number of distinct images transformed so far.
    pub fn cached_images(&self) -> usize {
        self.cache.lock().unwrap().len()
    }

    /// Check, transform and cache a guest program for a given CPU.
    /// Errors are ABI error codes (`E_INVAL`, `E_VERSION`, `E_UNSUPPORTED`).
    pub fn load_image(&self, raw: &[u8], cpu: &CpuSpec) -> Result<Arc<Image>, i32> {
        abi::check_abi_section(raw)?;
        abi::check_features(raw, cpu)?;
        self.load_unchecked(raw)
    }

    /// Transform without ABI checks (used on restore, where the image was
    /// accepted before it was saved).
    pub(crate) fn load_unchecked(&self, raw: &[u8]) -> Result<Arc<Image>, i32> {
        let hash = weave_core::sha256::sha256(raw);
        if let Some(img) = self.cache.lock().unwrap().get(&hash) {
            return Ok(img.clone());
        }
        let module = WeaveModule::from_raw(raw, &self.transform).map_err(|_| abi::E_INVAL)?;
        let img = Arc::new(Image {
            hash,
            raw: Arc::new(raw.to_vec()),
            module,
        });
        self.cache.lock().unwrap().insert(hash, img.clone());
        Ok(img)
    }
}
