//! Generates Rust types for `proto/common/v1/common.proto`.
//!
//! The proto directory lives at the repository root, three levels above this
//! crate (`services/rust/crates/common`), so both input paths are relative to
//! `CARGO_MANIFEST_DIR`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure().compile_protos(
        &["../../../proto/common/v1/common.proto"],
        &["../../../proto"],
    )?;

    println!("cargo:rerun-if-changed=../../../proto/common/v1/common.proto");
    Ok(())
}
