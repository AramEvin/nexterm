/**
 * TypeScript mirrors of the Rust DTOs that cross the IPC boundary.
 * Keep these in sync with `src-tauri/src` (field names are camelCase on the wire).
 */

/** Error codes returned by `AppError::code()` in `src-tauri/src/error.rs`. */
export type AppErrorCode = "VALIDATION" | "NOT_FOUND" | "IO" | "INTERNAL";

/** Wire format of a serialized `AppError`. */
export interface AppErrorPayload {
  code: AppErrorCode;
  message: string;
}

/** `commands::app::AppInfo` */
export interface AppInfo {
  name: string;
  version: string;
  logDir: string;
}
