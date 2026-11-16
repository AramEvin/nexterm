fn main() {
    // Declaring app commands here makes Tauri generate a permission for each one
    // (e.g. `allow-get-app-info`). Commands are then callable ONLY if a capability
    // file grants them - deny-by-default instead of Tauri's allow-all default.
    let attributes =
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
            "get_app_info",
            "library_list",
            "folder_create",
            "folder_rename",
            "folder_move",
            "folder_delete",
            "session_create",
            "session_update",
            "session_delete",
            "session_duplicate",
            "session_move",
            "session_set_favorite",
            "session_mark_connected",
            "ssh_connect",
            "ssh_write",
            "ssh_resize",
            "ssh_disconnect",
            "ssh_reconnect",
            "ssh_duplicate",
            "ssh_forget",
            "ssh_connect_saved",
            "ssh_prompt_respond",
            "vault_status",
            "vault_setup",
            "vault_unlock",
            "vault_lock",
            "vault_forget_device",
            "vault_change_password",
            "vault_set_auto_lock",
            "vault_touch",
            "credential_list",
            "credential_create_password",
            "credential_import_key",
            "credential_update",
            "credential_delete",
            "folder_set_default_credential",
            "known_hosts_list",
            "known_hosts_remove",
        ]));

    if let Err(err) = tauri_build::try_build(attributes) {
        panic!("tauri-build failed: {err:#}");
    }
}
