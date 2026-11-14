/** Mirrors `src-tauri/src/storage/models.rs` (camelCase on the wire). */

export type Protocol = "ssh";

export interface Folder {
  id: string;
  parentId: string | null;
  name: string;
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
  authMethod: string;
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
