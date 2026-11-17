/** Mirrors of the file-manager DTOs in `src-tauri/src/sftp` and `commands/sftp.rs`. */

/** Which side a file command is about. */
export type Endpoint = { kind: "local" } | { kind: "remote"; fsId: string };

export type EntryKind = "file" | "dir" | "symlink" | "other";

/** `sftp::FileEntry` */
export interface FileEntry {
  name: string;
  path: string;
  kind: EntryKind;
  linkToDir: boolean;
  size: number;
  /** Unix seconds. */
  modified: number | null;
  /** POSIX mode bits, e.g. 0o755; null on Windows disks. */
  permissions: number | null;
  owner: string | null;
  group: string | null;
  hidden: boolean;
}

/** `sftp::Listing` */
export interface Listing {
  /** "" = the drive list on Windows. */
  path: string;
  parent: string | null;
  entries: FileEntry[];
  truncated: boolean;
}

/** `commands::sftp::FsInfo` */
export interface FsInfo {
  fsId: string;
  home: string;
  label: string;
  protocol: "sftp" | "ftp" | "ftps";
  /** False for plain FTP. */
  encrypted: boolean;
}

/** `sftp::edit::EditEvent` */
export type EditEvent =
  | { type: "opened"; localPath: string; editor: string }
  | { type: "uploaded"; bytes: number; atMs: number }
  | { type: "failed"; message: string }
  | { type: "closed" };

export type Conflict = "overwrite" | "skip" | "rename";

/** `commands::sftp::TransferRequest` */
export interface TransferRequest {
  source: Endpoint;
  sources: string[];
  target: Endpoint;
  targetDir: string;
  conflict: Conflict;
}

/** `sftp::transfer::TransferEvent` */
export type TransferEvent =
  | { type: "queued" }
  | { type: "started"; filesTotal: number; bytesTotal: number }
  | {
      type: "progress";
      filesDone: number;
      filesTotal: number;
      bytesDone: number;
      bytesTotal: number;
      bytesPerSec: number;
      current: string;
    }
  | {
      type: "done";
      filesDone: number;
      bytesDone: number;
      skipped: number;
      skippedNames: string[];
    }
  | { type: "failed"; message: string; filesDone: number; bytesDone: number }
  | { type: "cancelled" };

export function isDirLike(e: FileEntry): boolean {
  return e.kind === "dir" || e.linkToDir;
}
