/**
 * TypeScript mirrors of the Rust DTOs that cross the IPC boundary.
 * Keep these in sync with `src-tauri/src` (field names are camelCase on the wire).
 */

/** Error codes returned by `AppError::code()` in `src-tauri/src/error.rs`. */
export type AppErrorCode =
  | "VALIDATION"
  | "NOT_FOUND"
  | "CONNECTION"
  | "AUTH_FAILED"
  | "VAULT_LOCKED"
  | "WRONG_MASTER_PASSWORD"
  | "HOST_KEY_CHANGED"
  | "HOST_KEY_REJECTED"
  | "CREDENTIALS_REQUIRED"
  | "CANCELLED"
  | "FILE"
  | "IO"
  | "INTERNAL";

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

/** `commands::ssh::SshConnectSavedRequest`: credentials are resolved in Rust. */
export interface SshConnectSavedRequest {
  sessionId: string;
  /** Only after a CREDENTIALS_REQUIRED answer. */
  username?: string;
  /** Only after a CREDENTIALS_REQUIRED answer; sent once, never stored. */
  password?: string;
  cols: number;
  rows: number;
}

/** `ssh::prompt::SshPrompt`: questions Rust asks during a connection. */
export type SshPrompt =
  | {
      type: "hostKey";
      promptId: string;
      /** "ssh" host key, or "tls" certificate of an FTPS server. */
      kind: "ssh" | "tls";
      host: string;
      port: number;
      keyType: string;
      fingerprint: string;
      otherKnownTypes: string[];
    }
  | {
      type: "keyboardInteractive";
      promptId: string;
      host: string;
      name: string;
      instructions: string;
      fields: { text: string; echo: boolean }[];
    };

export type HostKeyDecision = "acceptOnce" | "acceptAndSave" | "reject";

/** `ssh::prompt::PromptResponse` */
export type PromptResponse =
  | { type: "hostKey"; decision: HostKeyDecision }
  | { type: "answers"; answers: string[] }
  | { type: "cancel" };

/** `vault::VaultStatus` */
export interface VaultStatus {
  initialized: boolean;
  unlocked: boolean;
  remembered: boolean;
  /** 0 = never */
  autoLockMinutes: number;
}

/** `vault::credentials::Credential` (metadata only; secrets never leave Rust). */
export interface Credential {
  id: string;
  name: string;
  kind: "password" | "key";
  username: string;
  keyAlgorithm: string | null;
  keyFingerprint: string | null;
  usedBy: number;
  createdAt: number;
  updatedAt: number;
}

/** `ssh::known_hosts::KnownHost` */
export interface KnownHost {
  host: string;
  port: number;
  keyType: string;
  fingerprint: string;
  addedAt: number;
}
