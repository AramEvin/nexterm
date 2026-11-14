/**
 * Typed wrappers around Tauri's `invoke()`.
 *
 * RULE: this is the ONLY file allowed to call `invoke`. Components and stores
 * call the functions exported here, so every IPC call is typed and every
 * error is normalized into an `IpcError`.
 */
import { Channel, invoke, type InvokeArgs } from "@tauri-apps/api/core";

import type {
  AppErrorCode,
  AppErrorPayload,
  AppInfo,
  SessionStatus,
  SshConnectRequest,
  SshSessionInfo,
} from "@/types/ipc";

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

/** Raw channel payloads arrive as ArrayBuffer; normalise to Uint8Array. */
function toBytes(payload: unknown): Uint8Array {
  if (payload instanceof ArrayBuffer) return new Uint8Array(payload);
  if (payload instanceof Uint8Array) return payload;
  if (Array.isArray(payload)) return Uint8Array.from(payload as number[]);
  return new Uint8Array();
}

// ---- app ------------------------------------------------------------------

export function getAppInfo(): Promise<AppInfo> {
  return call<AppInfo>("get_app_info");
}

// ---- ssh ------------------------------------------------------------------

export interface SshSessionCallbacks {
  /** Raw terminal output (UTF-8 bytes, possibly split mid-character). */
  onOutput: (bytes: Uint8Array) => void;
  onStatus: (status: SessionStatus) => void;
}

function sessionChannels(callbacks: SshSessionCallbacks) {
  const onOutput = new Channel<unknown>((payload) => {
    callbacks.onOutput(toBytes(payload));
  });
  const onStatus = new Channel<SessionStatus>(callbacks.onStatus);
  return { onOutput, onStatus };
}

/** Connect and start a shell in a new tab. */
export function sshConnect(
  request: SshConnectRequest,
  callbacks: SshSessionCallbacks,
): Promise<SshSessionInfo> {
  return call<SshSessionInfo>("ssh_connect", { request, ...sessionChannels(callbacks) });
}

/** New session for an existing tab, using the params remembered in Rust. */
export function sshReconnect(
  connectionId: string,
  cols: number,
  rows: number,
  callbacks: SshSessionCallbacks,
): Promise<SshSessionInfo> {
  return call<SshSessionInfo>("ssh_reconnect", {
    connectionId,
    cols,
    rows,
    ...sessionChannels(callbacks),
  });
}

/** Same connection in a new tab; the result has a new `connectionId`. */
export function sshDuplicate(
  connectionId: string,
  cols: number,
  rows: number,
  callbacks: SshSessionCallbacks,
): Promise<SshSessionInfo> {
  return call<SshSessionInfo>("ssh_duplicate", {
    connectionId,
    cols,
    rows,
    ...sessionChannels(callbacks),
  });
}

/** Tab closed: wipe the remembered params (incl. password) in Rust. */
export function sshForget(connectionId: string): Promise<void> {
  return call<undefined>("ssh_forget", { connectionId });
}

export function sshWrite(sessionId: string, data: string): Promise<void> {
  return call<undefined>("ssh_write", { sessionId, data });
}

export function sshResize(sessionId: string, cols: number, rows: number): Promise<void> {
  return call<undefined>("ssh_resize", { sessionId, cols, rows });
}

export function sshDisconnect(sessionId: string): Promise<void> {
  return call<undefined>("ssh_disconnect", { sessionId });
}
