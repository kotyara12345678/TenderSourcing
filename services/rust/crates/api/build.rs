use std::env;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let proto_root = manifest_dir.join("../../../../proto");
    let common_file = proto_root.join("common/v1/common.proto");
    let documents_file = proto_root.join("documents/v1/document.proto");
    println!("cargo:rerun-if-changed={}", common_file.display());
    println!("cargo:rerun-if-changed={}", documents_file.display());
    tonic_prost_build::configure()
        .compile_protos(&[&common_file, &documents_file], &[&proto_root])?;
    Ok(())
}
