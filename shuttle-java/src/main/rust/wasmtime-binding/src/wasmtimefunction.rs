use crate::wasminstance::JWasmtimeInstance;
use jni::bind_java_type;

bind_java_type! {
    rust_type = pub JWasmtimeFunction,
    java_type = "jp.akkiserver.shuttle.WasmtimeFunction",

    type_map = {
        JWasmtimeInstance => "jp.akkiserver.shuttle.WasmtimeInstance",
    },

    methods = {
        fn call(instance: JWasmtimeInstance, args: JObject[]) -> JObject[],
    }
}
