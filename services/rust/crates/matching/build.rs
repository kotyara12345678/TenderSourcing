use std::env;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let proto_root = manifest_dir.join("../../../../proto");
    let proto_file = proto_root.join("matching/v1/matching.proto");
    let common_file = proto_root.join("common/v1/common.proto");
    println!("cargo:rerun-if-changed={}", proto_file.display());
    println!("cargo:rerun-if-changed={}", common_file.display());
    tonic_prost_build::configure().compile_protos(&[&proto_file], &[&proto_root])?;
    Ok(())
}
