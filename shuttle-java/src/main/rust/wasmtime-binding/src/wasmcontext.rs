use jni::bind_java_type;

bind_java_type! {
    rust_type = pub JWasmContext,
    java_type = "jp.akkiserver.shuttle.WasmContext",


}
