fn main() {
    // Declaring app commands here makes Tauri generate a permission for each one
    // (e.g. `allow-get-app-info`). Commands are then callable ONLY if a capability
    // file grants them - deny-by-default instead of Tauri's allow-all default.
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(&["get_app_info"]));

    if let Err(err) = tauri_build::try_build(attributes) {
        panic!("tauri-build failed: {err:#}");
    }
}
