//! App-level commands (version info, diagnostics).

use serde::Serialize;
use tauri::State;

use crate::error::AppResult;
use crate::logging::LogGuard;

/// Basic application info shown in the status bar / About dialog.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub log_dir: String,
}

#[tauri::command]
pub fn get_app_info(logs: State<'_, LogGuard>) -> AppResult<AppInfo> {
    Ok(AppInfo {
        name: "NexTerm",
        version: env!("CARGO_PKG_VERSION"),
        log_dir: logs.log_dir.display().to_string(),
    })
}
