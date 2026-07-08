fn main() {
    // Windows test binaries need the Common-Controls v6 manifest the app
    // binary gets from tauri-build's winres step; without it every test exe
    // dies at DLL load with STATUS_ENTRYPOINT_NOT_FOUND before a single test
    // runs (tauri-apps/tauri#13419). Scoped to test targets via
    // rustc-link-arg-tests so it can never fight the app's own embedded
    // manifest resource.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        let manifest =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-test-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }

    tauri_build::build()
}
