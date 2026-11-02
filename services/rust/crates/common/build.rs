//! Generates Rust types for `proto/common/v1/common.proto`.
//!
//! Cargo runs build scripts with the working directory set to this crate's
//! manifest dir (`services/rust/crates/common`), which is four levels below
//! the repository root where `proto/` lives. Paths are anchored at
//! `CARGO_MANIFEST_DIR` so they do not depend on the caller's cwd.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")?;
    let proto_root = std::path::Path::new(&manifest_dir).join("../../../../proto");
    let proto_file = proto_root.join("common/v1/common.proto");

    tonic_prost_build::configure().compile_protos([&proto_file], [&proto_root])?;

    println!("cargo:rerun-if-changed={}", proto_file.display());
    Ok(())
}
