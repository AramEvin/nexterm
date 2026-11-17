/** Mirrors `src-tauri/src/storage/models.rs` (camelCase on the wire). */

export type Protocol = "ssh" | "ftp" | "ftps";

/** FTP/FTPS sessions open in the file manager, not a terminal. */
export function isFileProtocol(p: Protocol): boolean {
  return p === "ftp" || p === "ftps";
}

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
