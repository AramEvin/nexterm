/**
 * Typed wrappers around Tauri's `invoke()`.
 *
 * RULE: this is the ONLY file allowed to call `invoke`. Components and stores
 * call the functions exported here, so every IPC call is typed and every
 * error is normalized into an `IpcError`.
 */
import { invoke, type InvokeArgs } from "@tauri-apps/api/core";

import type { AppErrorCode, AppErrorPayload, AppInfo } from "@/types/ipc";

/** Error thrown by every IPC wrapper. `code` mirrors the Rust `AppError` code. */
export class IpcError extends Error {
  readonly code: AppErrorCode;

  constructor(code: AppErrorCode, message: string) {
    super(message);
    this.name = "IpcError";
    this.code = code;
  }
}

function isAppErrorPayload(value: unknown): value is AppErrorPayload {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    "message" in value &&
    typeof (value as { code: unknown }).code === "string" &&
    typeof (value as { message: unknown }).message === "string"
  );
}

async function call<T>(command: string, args?: InvokeArgs): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (err: unknown) {
    if (isAppErrorPayload(err)) {
      throw new IpcError(err.code, err.message);
    }
    // Tauri-level failures (e.g. command not permitted) arrive as plain strings.
    throw new IpcError("INTERNAL", typeof err === "string" ? err : "IPC call failed");
  }
}

// ---- app ------------------------------------------------------------------

export function getAppInfo(): Promise<AppInfo> {
  return call<AppInfo>("get_app_info");
}
