/** Mirrors `src-tauri/src/storage/models.rs` (camelCase on the wire). */

export type Protocol = "ssh" | "ftp" | "ftps" | "telnet" | "serial" | "local";

/** FTP/FTPS sessions open in the file manager, not a terminal. */
export function isFileProtocol(p: Protocol): boolean {
  return p === "ftp" || p === "ftps";
}

/** Uses host and port (not a COM port or a local shell). */
export function isNetworkProtocol(p: Protocol): boolean {
  return p !== "serial" && p !== "local";
}

/** Has a username / password / key login handled by NexTerm. */
export function hasLogin(p: Protocol): boolean {
  return p === "ssh" || p === "ftp" || p === "ftps";
}

/** Sends everything in clear text. */
export function isUnencrypted(p: Protocol): boolean {
  return p === "ftp" || p === "telnet";
}

export type Parity = "none" | "odd" | "even";
export type FlowControl = "none" | "software" | "hardware";

/** `session::options::SerialOptions` */
export interface SerialOptions {
  port: string;
  baudRate: number;
  dataBits: number;
  parity: Parity;
  stopBits: number;
  flowControl: FlowControl;
}

export type LocalShell = "powershell" | "pwsh" | "cmd" | "wsl" | "default";

/** `session::options::LocalOptions` */
export interface LocalOptions {
  shell: LocalShell;
  /** WSL distribution; empty = the default one. */
  wslDistro: string;
}

/** `session::options::SessionOptions` */
export interface SessionOptions {
  serial?: SerialOptions;
  local?: LocalOptions;
}

export interface LocalShellInfo {
  shell: LocalShell;
  label: string;
  available: boolean;
}

export const DEFAULT_SERIAL: SerialOptions = {
  port: "",
  baudRate: 9600,
  dataBits: 8,
  parity: "none",
  stopBits: 1,
  flowControl: "none",
};

export type AuthMethod = "password" | "publickey" | "agent" | "keyboard-interactive";

export interface Folder {
  id: string;
  parentId: string | null;
  name: string;
  /** Credential used by sessions inside that have none of their own. */
  defaultCredentialId: string | null;
  sortOrder: number;
  createdAt: number;
  updatedAt: number;
}

export interface SavedSession {
  id: string;
  folderId: string | null;
  name: string;
  protocol: Protocol;
  host: string;
  port: number;
  username: string;
  authMethod: AuthMethod;
  credentialId: string | null;
  jumpHostId: string | null;
  color: string | null;
  notes: string;
  favorite: boolean;
  /** Live CPU / RAM / disk bar under the terminal (SSH). */
  resourceMonitor: boolean;
  options: SessionOptions;
  tags: string[];
  sortOrder: number;
  createdAt: number;
  updatedAt: number;
  lastConnectedAt: number | null;
}

/** Editable fields (create / update). */
export interface SessionInput {
  folderId: string | null;
  name: string;
  protocol: Protocol;
  host: string;
  port: number;
  username: string;
  authMethod: AuthMethod;
  /** `null` = inherit the folder default, or ask for a password. */
  credentialId: string | null;
  color: string | null;
  notes: string;
  favorite: boolean;
  /** Live CPU / RAM / disk bar under the terminal (SSH). */
  resourceMonitor: boolean;
  /** Saved SSH session to connect through (ProxyJump); `null` = direct. */
  jumpHostId: string | null;
  /** Serial line settings or local shell choice. */
  options: SessionOptions;
  tags: string[];
}

export interface Library {
  folders: Folder[];
  sessions: SavedSession[];
}

export interface MoveRequest {
  id: string;
  parentId: string | null;
  beforeId: string | null;
}

/** `import::ImportSource` */
export type ImportSource = "mobaxterm" | "putty";

/** Where an import reads from (`commands::import::ImportFrom`). */
export type ImportFrom = "mobaxterm-file" | "putty-file" | "putty-registry";

export interface ImportedHop {
  host: string;
  port: number;
  username: string;
}

/** A session found in the other program's file (`import::ImportedSession`). */
export interface ImportedSession {
  name: string;
  folder: string[];
  protocol: Protocol;
  host: string;
  port: number;
  username: string;
  authMethod: AuthMethod;
  keyPath: string | null;
  /** Jump hosts, dialled first to last. */
  jump: ImportedHop[];
  options: SessionOptions;
}

export interface ImportPreview {
  source: ImportSource;
  sessions: ImportedSession[];
  skipped: { name: string; reason: string }[];
}

export interface ImportResult {
  folderId: string;
  created: number;
  jumpHosts: number;
  existing: number;
  failed: number;
}
