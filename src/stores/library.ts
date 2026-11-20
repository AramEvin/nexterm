/**
 * Saved sessions and folders (the sidebar "library").
 *
 * Source of truth is SQLite in Rust; this store holds a copy for instant
 * rendering and search. Every mutation goes to Rust first and then reloads
 * the list, so the UI never shows state the database doesn't have.
 */
import { create } from "zustand";

import {
  folderCreate,
  folderDelete,
  folderMove,
  folderRename,
  IpcError,
  libraryList,
  sessionCreate,
  sessionDelete,
  sessionDuplicate,
  sessionMove,
  sessionSetFavorite,
  sessionUpdate,
} from "@/lib/ipc";
import type { Folder, SavedSession, SessionInput } from "@/types/library";

const EXPANDED_KEY = "nexterm.library.expanded";

/** Expanded folders are a per-device view preference; storage may be unavailable. */
function loadExpanded(): Set<string> {
  try {
    const raw = localStorage.getItem(EXPANDED_KEY);
    const ids: unknown = raw ? JSON.parse(raw) : [];
    return new Set(Array.isArray(ids) ? ids.filter((x): x is string => typeof x === "string") : []);
  } catch {
    return new Set();
  }
}

function saveExpanded(ids: Set<string>): void {
  try {
    localStorage.setItem(EXPANDED_KEY, JSON.stringify([...ids]));
  } catch {
    // not fatal: folders just start collapsed next time
  }
}

interface LibraryState {
  folders: Folder[];
  sessions: SavedSession[];
  loaded: boolean;
  /** Set when the database can't be read (shown in the sidebar). */
  error: string | null;
  query: string;
  expanded: Set<string>;
  /** Folder currently being renamed inline (e.g. right after "New folder"). */
  renamingFolderId: string | null;
  setRenamingFolder: (id: string | null) => void;

  load: () => Promise<void>;
  setQuery: (q: string) => void;
  toggleFolder: (id: string) => void;
  expandFolder: (id: string) => void;

  createFolder: (parentId: string | null, name: string) => Promise<Folder>;
  /** Create "New folder" under `parentId` and start renaming it inline. */
  newFolder: (parentId: string | null) => Promise<void>;
  renameFolder: (id: string, name: string) => Promise<void>;
  moveFolder: (id: string, parentId: string | null, beforeId?: string | null) => Promise<void>;
  deleteFolder: (id: string) => Promise<number>;

  createSession: (input: SessionInput) => Promise<SavedSession>;
  updateSession: (id: string, input: SessionInput) => Promise<SavedSession>;
  deleteSession: (id: string) => Promise<void>;
  duplicateSession: (id: string) => Promise<SavedSession>;
  moveSession: (id: string, folderId: string | null, beforeId?: string | null) => Promise<void>;
  setFavorite: (id: string, favorite: boolean) => Promise<void>;
}

export const useLibraryStore = create<LibraryState>()((set, get) => {
  /** Run a mutation, then reload. Errors propagate to the caller (dialogs show them). */
  const mutate = async <T>(op: () => Promise<T>): Promise<T> => {
    try {
      return await op();
    } finally {
      await get().load();
    }
  };

  return {
    folders: [],
    sessions: [],
    loaded: false,
    error: null,
    query: "",
    expanded: loadExpanded(),
    renamingFolderId: null,

    setRenamingFolder: (renamingFolderId) => {
      set({ renamingFolderId });
    },

    load: async () => {
      try {
        const lib = await libraryList();
        set({ folders: lib.folders, sessions: lib.sessions, loaded: true, error: null });
      } catch (err) {
        set({
          loaded: true,
          error: err instanceof IpcError ? err.message : "Could not load saved sessions.",
        });
      }
    },

    setQuery: (query) => {
      set({ query });
    },

    toggleFolder: (id) => {
      const expanded = new Set(get().expanded);
      if (expanded.has(id)) expanded.delete(id);
      else expanded.add(id);
      saveExpanded(expanded);
      set({ expanded });
    },

    expandFolder: (id) => {
      if (get().expanded.has(id)) return;
      const expanded = new Set(get().expanded).add(id);
      saveExpanded(expanded);
      set({ expanded });
    },

    createFolder: (parentId, name) =>
      mutate(async () => {
        const folder = await folderCreate(parentId, name);
        if (parentId) get().expandFolder(parentId);
        return folder;
      }),

    newFolder: async (parentId) => {
      const folder = await get().createFolder(parentId, "New folder");
      set({ renamingFolderId: folder.id });
    },

    renameFolder: (id, name) =>
      mutate(async () => {
        await folderRename(id, name);
      }),

    moveFolder: (id, parentId, beforeId = null) =>
      mutate(async () => {
        await folderMove({ id, parentId, beforeId });
        if (parentId) get().expandFolder(parentId);
      }),

    deleteFolder: (id) => mutate(() => folderDelete(id)),

    createSession: (input) =>
      mutate(async () => {
        const session = await sessionCreate(input);
        if (input.folderId) get().expandFolder(input.folderId);
        return session;
      }),

    updateSession: (id, input) => mutate(() => sessionUpdate(id, input)),

    deleteSession: (id) => mutate(() => sessionDelete(id)),

    duplicateSession: (id) => mutate(() => sessionDuplicate(id)),

    moveSession: (id, folderId, beforeId = null) =>
      mutate(async () => {
        await sessionMove({ id, parentId: folderId, beforeId });
        if (folderId) get().expandFolder(folderId);
      }),

    setFavorite: (id, favorite) =>
      mutate(async () => {
        await sessionSetFavorite(id, favorite);
      }),
  };
});

/** Editable fields of an existing session, for pre-filling the editor. */
export function toInput(s: SavedSession): SessionInput {
  return {
    folderId: s.folderId,
    name: s.name,
    protocol: s.protocol,
    host: s.host,
    port: s.port,
    username: s.username,
    authMethod: s.authMethod,
    credentialId: s.credentialId,
    color: s.color,
    notes: s.notes,
    favorite: s.favorite,
    resourceMonitor: s.resourceMonitor,
    jumpHostId: s.jumpHostId,
    tags: s.tags,
  };
}
