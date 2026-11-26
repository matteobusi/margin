use std::collections::HashMap;

fn main() {
    // MARGIN_PREBUILT_GUEST=<path to margin-guest.bin>: embed an already-built guest instead of
    // building it. Release builds use this so every platform embeds the same, reproducibly
    // built guest (and therefore the same image ID).
    println!("cargo:rerun-if-env-changed=MARGIN_PREBUILT_GUEST");
    if let Ok(path) = std::env::var("MARGIN_PREBUILT_GUEST") {
        let path = std::path::Path::new(&path).canonicalize().expect("MARGIN_PREBUILT_GUEST path");
        println!("cargo:rerun-if-changed={}", path.display());
        let blob = std::fs::read(&path).expect("read prebuilt guest");
        let id = risc0_binfmt::compute_image_id(&blob).expect("compute image id");
        let words: Vec<String> = id.as_words().iter().map(|w| format!("{w}")).collect();
        let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("methods.rs");
        std::fs::write(
            out,
            format!(
                "pub const MARGIN_GUEST_ELF: &[u8] = include_bytes!({p:?});\npub const MARGIN_GUEST_ID: [u32; 8] = [{w}];\npub const MARGIN_GUEST_PATH: &str = {p:?};\n",
                p = path.display().to_string(),
                w = words.join(", ")
            ),
        )
        .unwrap();
        return;
    }

    // Forward the kernel choice to the guest build.
    let mut features = Vec::new();
    if std::env::var("CARGO_FEATURE_KERNEL_NANODA").is_ok() {
        features.push("kernel-nanoda".to_string());
    }
    if std::env::var("CARGO_FEATURE_UNSOUND_SKIP_TYPECHECK").is_ok() {
        features.push("unsound-skip-typecheck".to_string());
    }
    let mut opts = risc0_build::GuestOptionsBuilder::default();
    opts.features(features);
    // MARGIN_REPRODUCIBLE=1: build the guest in RISC Zero's pinned Docker image, so anyone
    // gets the same image ID. Release image IDs (checkers.json) must come from this.
    println!("cargo:rerun-if-env-changed=MARGIN_REPRODUCIBLE");
    if std::env::var("MARGIN_REPRODUCIBLE").as_deref() == Ok("1") {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root");
        opts.use_docker(risc0_build::DockerOptionsBuilder::default().root_dir(root).build().expect("docker options"));
    }
    let opts = opts.build().expect("guest options");
    risc0_build::embed_methods_with_options(HashMap::from([("margin-guest", opts)]));
}
