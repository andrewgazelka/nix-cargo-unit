fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let proto_path = std::path::Path::new(&manifest_dir).join("proto/hello.proto");

    tonic_build::compile_protos(&proto_path).expect("failed to compile protos");
}
