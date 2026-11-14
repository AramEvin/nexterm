/**
 * TypeScript mirrors of the Rust DTOs that cross the IPC boundary.
 * Keep these in sync with `src-tauri/src` (field names are camelCase on the wire).
 */

/** Error codes returned by `AppError::code()` in `src-tauri/src/error.rs`. */
export type AppErrorCode =
  "VALIDATION" | "NOT_FOUND" | "CONNECTION" | "AUTH_FAILED" | "IO" | "INTERNAL";

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

/** `commands::ssh::SshConnectRequest` */
export interface SshConnectRequest {
  host: string;
  port: number;
  username: string;
  /** Sent once, in this request only. Never stored in frontend state. */
  password: string;
  cols: number;
  rows: number;
}

/** `commands::ssh::SshSessionInfo` */
export interface SshSessionInfo {
  /** The live session; changes on every reconnect. */
  sessionId: string;
  /** Opaque handle to connection params kept in Rust; owned by one tab. */
  connectionId: string;
}

/** `session::protocol::SessionStatus` */
export type SessionStatus = { kind: "closed"; reason: string };
