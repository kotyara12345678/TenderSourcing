use std::env;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let proto_root = manifest_dir.join("../../../../proto");
    let protos = [
        "common/v1/common.proto",
        "documents/v1/document.proto",
        "ai/v1/ai.proto",
        "search/v1/search.proto",
        "reports/v1/report.proto",
        "matching/v1/matching.proto",
    ];
    let mut files = Vec::with_capacity(protos.len());
    for proto in protos {
        let file = proto_root.join(proto);
        println!("cargo:rerun-if-changed={}", file.display());
        files.push(file);
    }
    let refs: Vec<&std::path::Path> = files.iter().map(PathBuf::as_path).collect();
    tonic_prost_build::configure().compile_protos(&refs, &[&proto_root])?;
    Ok(())
}
