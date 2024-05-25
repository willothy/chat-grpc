fn main() {
    println!("cargo:rerun-if-changed=proto/streaming.proto");
    tonic_build::compile_protos("proto/streaming.proto").unwrap();
}
