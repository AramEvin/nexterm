/**
 * Typed wrappers around Tauri's `invoke()`.
 *
 * RULE: this is the ONLY file allowed to call `invoke`. Components and stores
 * call the functions exported here, so every IPC call is typed and every
 * error is normalized into an `IpcError`.
 */
import { Channel, invoke, type InvokeArgs } from "@tauri-apps/api/core";

import type {
  Credential,
  KnownHost,
  PromptResponse,
  SshConnectSavedRequest,
  SshPrompt,
  VaultStatus,
  AppErrorCode,
  AppErrorPayload,
  AppInfo,
  SessionStatus,
  SshConnectRequest,
  SshSessionInfo,
} from "@/types/ipc";
import type {
  EditEvent,
  Endpoint,
  FsInfo,
  Listing,
  TransferEvent,
  TransferRequest,
} from "@/types/files";
import type { Folder, Library, MoveRequest, SavedSession, SessionInput } from "@/types/library";

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
  /** Host-key or 2FA question; answer with `sshPromptRespond`. */
  onPrompt: (prompt: SshPrompt) => void;
}

function sessionChannels(callbacks: SshSessionCallbacks) {
  const onOutput = new Channel<unknown>((payload) => {
    callbacks.onOutput(toBytes(payload));
  });
  const onStatus = new Channel<SessionStatus>(callbacks.onStatus);
  const onPrompt = new Channel<SshPrompt>(callbacks.onPrompt);
  return { onOutput, onStatus, onPrompt };
}

/** Connect and start a shell in a new tab. */
export function sshConnect(
  request: SshConnectRequest,
  callbacks: SshSessionCallbacks,
): Promise<SshSessionInfo> {
  return call<SshSessionInfo>("ssh_connect", { request, ...sessionChannels(callbacks) });
}

/** Connect to a saved session; Rust resolves the credential from the vault. */
export function sshConnectSaved(
  request: SshConnectSavedRequest,
  callbacks: SshSessionCallbacks,
): Promise<SshSessionInfo> {
  return call<SshSessionInfo>("ssh_connect_saved", { request, ...sessionChannels(callbacks) });
}

/** Answer a host-key or keyboard-interactive prompt. */
export function sshPromptRespond(promptId: string, response: PromptResponse): Promise<void> {
  return call<undefined>("ssh_prompt_respond", { promptId, response });
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

// ---- library (saved sessions & folders) -----------------------------------

export function libraryList(): Promise<Library> {
  return call<Library>("library_list");
}

export function folderCreate(parentId: string | null, name: string): Promise<Folder> {
  return call<Folder>("folder_create", { parentId, name });
}

export function folderRename(id: string, name: string): Promise<Folder> {
  return call<Folder>("folder_rename", { id, name });
}

export function folderMove(request: MoveRequest): Promise<void> {
  return call<undefined>("folder_move", { request });
}

/** Resolves with the number of sessions deleted with the folder. */
export function folderDelete(id: string): Promise<number> {
  return call<number>("folder_delete", { id });
}

export function sessionCreate(input: SessionInput): Promise<SavedSession> {
  return call<SavedSession>("session_create", { input });
}

export function sessionUpdate(id: string, input: SessionInput): Promise<SavedSession> {
  return call<SavedSession>("session_update", { id, input });
}

export function sessionDelete(id: string): Promise<void> {
  return call<undefined>("session_delete", { id });
}

export function sessionDuplicate(id: string): Promise<SavedSession> {
  return call<SavedSession>("session_duplicate", { id });
}

export function sessionMove(request: MoveRequest): Promise<void> {
  return call<undefined>("session_move", { request });
}

export function sessionSetFavorite(id: string, favorite: boolean): Promise<void> {
  return call<undefined>("session_set_favorite", { id, favorite });
}

export function sessionMarkConnected(id: string): Promise<void> {
  return call<undefined>("session_mark_connected", { id });
}

// ---- vault ----------------------------------------------------------------

export function vaultStatus(): Promise<VaultStatus> {
  return call<VaultStatus>("vault_status");
}

export function vaultSetup(masterPassword: string, remember: boolean): Promise<VaultStatus> {
  return call<VaultStatus>("vault_setup", { masterPassword, remember });
}

export function vaultUnlock(masterPassword: string, remember: boolean): Promise<VaultStatus> {
  return call<VaultStatus>("vault_unlock", { masterPassword, remember });
}

export function vaultLock(): Promise<VaultStatus> {
  return call<VaultStatus>("vault_lock");
}

export function vaultForgetDevice(): Promise<VaultStatus> {
  return call<VaultStatus>("vault_forget_device");
}

export function vaultChangePassword(oldPassword: string, newPassword: string): Promise<void> {
  return call<undefined>("vault_change_password", { oldPassword, newPassword });
}

export function vaultSetAutoLock(minutes: number): Promise<VaultStatus> {
  return call<VaultStatus>("vault_set_auto_lock", { minutes });
}

export function vaultTouch(): Promise<void> {
  return call<undefined>("vault_touch");
}

// ---- credentials (metadata only) -------------------------------------------

export function credentialList(): Promise<Credential[]> {
  return call<Credential[]>("credential_list");
}

export function credentialCreatePassword(
  name: string,
  username: string,
  password: string,
): Promise<Credential> {
  return call<Credential>("credential_create_password", { name, username, password });
}

/** Opens a native file picker (from Rust). Resolves `null` if cancelled. */
export function credentialImportKey(
  name: string,
  username: string,
  passphrase: string | null,
): Promise<Credential | null> {
  return call<Credential | null>("credential_import_key", { name, username, passphrase });
}

export function credentialUpdate(
  id: string,
  name: string,
  username: string,
  password: string | null,
): Promise<Credential> {
  return call<Credential>("credential_update", { id, name, username, password });
}

export function credentialDelete(id: string): Promise<void> {
  return call<undefined>("credential_delete", { id });
}

export function folderSetDefaultCredential(
  id: string,
  credentialId: string | null,
): Promise<Folder> {
  return call<Folder>("folder_set_default_credential", { id, credentialId });
}

// ---- known hosts ------------------------------------------------------------

export function knownHostsList(): Promise<KnownHost[]> {
  return call<KnownHost[]>("known_hosts_list");
}

export function knownHostsRemove(host: string, port: number): Promise<number> {
  return call<number>("known_hosts_remove", { host, port });
}

// ---- file manager -----------------------------------------------------------

/** Open a saved session in a file-manager pane (SFTP). */
export function sftpOpenSaved(
  request: { sessionId: string; username?: string; password?: string },
  onPrompt: (prompt: SshPrompt) => void,
): Promise<FsInfo> {
  return call<FsInfo>("sftp_open_saved", {
    request,
    onPrompt: new Channel<SshPrompt>(onPrompt),
  });
}

/** Open the server of a terminal tab (reuses its remembered login). */
export function sftpOpenConnection(
  connectionId: string,
  onPrompt: (prompt: SshPrompt) => void,
): Promise<FsInfo> {
  return call<FsInfo>("sftp_open_connection", {
    connectionId,
    onPrompt: new Channel<SshPrompt>(onPrompt),
  });
}

export function sftpClose(fsId: string): Promise<void> {
  return call<undefined>("sftp_close", { fsId });
}

export function fsHome(endpoint: Endpoint): Promise<string> {
  return call<string>("fs_home", { endpoint });
}

export function fsList(endpoint: Endpoint, path: string): Promise<Listing> {
  return call<Listing>("fs_list", { endpoint, path });
}

export function fsMkdir(endpoint: Endpoint, dir: string, name: string): Promise<string> {
  return call<string>("fs_mkdir", { endpoint, dir, name });
}

export function fsRename(endpoint: Endpoint, path: string, newName: string): Promise<string> {
  return call<string>("fs_rename", { endpoint, path, newName });
}

export function fsDelete(endpoint: Endpoint, paths: string[]): Promise<number> {
  return call<number>("fs_delete", { endpoint, paths });
}

export function fsChmod(endpoint: Endpoint, paths: string[], mode: number): Promise<void> {
  return call<undefined>("fs_chmod", { endpoint, paths, mode });
}

/** Queue a copy; returns the transfer id. */
export function transferStart(
  request: TransferRequest,
  onEvent: (event: TransferEvent) => void,
): Promise<string> {
  return call<string>("transfer_start", {
    request,
    onEvent: new Channel<TransferEvent>(onEvent),
  });
}

export function transferCancel(id: string): Promise<boolean> {
  return call<boolean>("transfer_cancel", { id });
}

/** Open a remote file in the local editor; saves are uploaded back. */
export function editStart(
  endpoint: Endpoint,
  path: string,
  onEvent: (event: EditEvent) => void,
): Promise<string> {
  return call<string>("edit_start", { endpoint, path, onEvent: new Channel<EditEvent>(onEvent) });
}

export function editStop(id: string): Promise<boolean> {
  return call<boolean>("edit_stop", { id });
}

/** Native folder picker; null if cancelled. */
export function pickLocalFolder(): Promise<string | null> {
  return call<string | null>("pick_local_folder");
}

/** Native multi-file picker; empty if cancelled. */
export function pickLocalFiles(): Promise<string[]> {
  return call<string[]>("pick_local_files");
}
