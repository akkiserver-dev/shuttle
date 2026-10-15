// jni 0.22.4's `bind_java_type!` expands to `AtomicBool::fetch_update`, deprecated
// since Rust 1.99. Remove once jni ships a fix (jni-rs/jni-rs#846).
#![allow(deprecated)]

pub mod java_numbers;
pub mod java_collections;
pub mod java_maps;


pub mod logging;
pub mod wasmcomponent;
pub mod wasmcomponentinstance;
pub mod wasmcomponentlinker;
pub mod wasmcomponentvalue;
pub mod wasmcontext;
pub mod wasmengine;
pub mod wasmexception;
pub mod wasminstance;
pub mod wasmlinker;
pub mod wasmmemory;
pub mod wasmmodule;
pub mod wasmsharedmemory;
pub mod wasmstore;
pub mod wasmtime_v128;
pub mod wasmtime_valtype;
pub mod wasmtimefuncref;
pub mod wasmtimefunction;
pub mod weave;
