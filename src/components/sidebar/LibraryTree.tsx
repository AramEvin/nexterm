import { useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import {
  ChevronRight,
  Copy,
  FolderClosed,
  FolderOpen,
  FolderPlus,
  KeyRound,
  Pencil,
  Plug,
  Plus,
  Star,
  StarOff,
  Trash2,
} from "lucide-react";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  buildIndex,
  countSessions,
  flattenTree,
  folderAndDescendants,
  folderPath,
  searchSessions,
  type LibraryIndex,
} from "@/lib/library";
import { openSavedSession } from "@/lib/connect";
import { cn } from "@/lib/utils";
import { useDialogStore } from "@/stores/dialogs";
import { useLibraryStore } from "@/stores/library";
import type { Folder, SavedSession } from "@/types/library";

const DRAG_THRESHOLD_PX = 5;
const INDENT_PX = 14;
/** Rendering more search hits than this only slows typing; ask to refine instead. */
const MAX_RESULTS = 300;

type DragItem = { kind: "folder" | "session"; id: string };
type DropTarget =
  { kind: "folder"; id: string } | { kind: "session"; id: string } | { kind: "root" };

/** Shared actions used by rows and the panel header. */
function useLibraryActions() {
  const openEditor = useDialogStore((s) => s.openEditor);
  const askConfirm = useDialogStore((s) => s.askConfirm);
  const deleteSession = useLibraryStore((s) => s.deleteSession);
  const deleteFolder = useLibraryStore((s) => s.deleteFolder);

  return {
    connect: (session: SavedSession) => {
      void openSavedSession(session);
    },
    edit: (session: SavedSession) => {
      openEditor({ mode: "edit", session });
    },
    newSession: (folderId: string | null) => {
      openEditor({ mode: "create", folderId });
    },
    confirmDeleteSession: (session: SavedSession) => {
      askConfirm({
        title: "Delete session?",
        description: `"${session.name}" will be removed from your saved sessions. Open tabs stay open.`,
        confirmLabel: "Delete",
        destructive: true,
        onConfirm: () => deleteSession(session.id),
      });
    },
    confirmDeleteFolder: (folder: Folder, count: number) => {
      askConfirm({
        title: "Delete folder?",
        description:
          count > 0
            ? `"${folder.name}" and everything in it (${count} saved session${count === 1 ? "" : "s"}, including subfolders) will be deleted.`
            : `The empty folder "${folder.name}" will be deleted.`,
        confirmLabel: count > 0 ? "Delete everything" : "Delete",
        destructive: true,
        onConfirm: async () => {
          await deleteFolder(folder.id);
        },
      });
    },
  };
}

/**
 * Pointer-based drag & drop (HTML5 DnD is unreliable inside WebView2 while
 * Tauri's native file drop is enabled). Drop on a folder = move into it, on a
 * session = place before it, on empty space = top level.
 */
function useTreeDrag(index: LibraryIndex) {
  const moveFolder = useLibraryStore((s) => s.moveFolder);
  const moveSession = useLibraryStore((s) => s.moveSession);
  const sessions = useLibraryStore((s) => s.sessions);
  const drag = useRef<{ item: DragItem; startY: number; active: boolean } | null>(null);
  const suppressClick = useRef(false);
  const [dragging, setDragging] = useState<DragItem | null>(null);
  const [target, setTarget] = useState<DropTarget | null>(null);
  const targetRef = useRef<DropTarget | null>(null);

  const isValid = (item: DragItem, t: DropTarget): boolean => {
    if (t.kind === "root") return true;
    if (item.kind === "folder") {
      // A folder may only be dropped onto folders that aren't itself or inside it.
      return t.kind === "folder" && !folderAndDescendants(index, item.id).has(t.id);
    }
    return !(t.kind === "session" && t.id === item.id);
  };

  const targetAt = (x: number, y: number): DropTarget | null => {
    const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-drop]");
    if (!el) return null;
    const kind = el.dataset.drop;
    const id = el.dataset.id;
    if (kind === "root") return { kind: "root" };
    if ((kind === "folder" || kind === "session") && id) return { kind, id };
    return null;
  };

  useEffect(() => {
    const onMove = (e: PointerEvent) => {
      const d = drag.current;
      if (!d) return;
      if (!d.active) {
        if (Math.abs(e.clientY - d.startY) < DRAG_THRESHOLD_PX) return;
        d.active = true;
        setDragging(d.item);
      }
      // The browser starts a text selection on press; a drag shouldn't paint one.
      document.getSelection()?.removeAllRanges();
      const t = targetAt(e.clientX, e.clientY);
      const valid = t && isValid(d.item, t) ? t : null;
      targetRef.current = valid;
      setTarget(valid);
    };
    const onUp = () => {
      const d = drag.current;
      drag.current = null;
      const t = targetRef.current;
      targetRef.current = null;
      setDragging(null);
      setTarget(null);
      if (!d?.active) return;
      suppressClick.current = true;
      if (!t) return;
      const op =
        d.item.kind === "folder"
          ? moveFolder(d.item.id, t.kind === "folder" ? t.id : null)
          : t.kind === "session"
            ? moveSession(d.item.id, sessions.find((s) => s.id === t.id)?.folderId ?? null, t.id)
            : moveSession(d.item.id, t.kind === "folder" ? t.id : null);
      void op.catch(() => undefined); // invalid moves are rejected by Rust; the list reloads
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("blur", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("blur", onUp);
    };
  });

  return {
    dragging,
    target,
    start: (item: DragItem) => (e: React.PointerEvent) => {
      if (e.button !== 0) return;
      drag.current = { item, startY: e.clientY, active: false };
    },
    /** True (once) if this click ends a drag and should be ignored. */
    consumeClick: () => {
      const was = suppressClick.current;
      suppressClick.current = false;
      return was;
    },
  };
}

export function LibraryTree() {
  const folders = useLibraryStore((s) => s.folders);
  const sessions = useLibraryStore((s) => s.sessions);
  const expanded = useLibraryStore((s) => s.expanded);
  const query = useLibraryStore((s) => s.query);
  const actions = useLibraryActions();
  const renamingId = useLibraryStore((s) => s.renamingFolderId);
  const setRenamingId = useLibraryStore((s) => s.setRenamingFolder);

  const index = useMemo(() => buildIndex(folders, sessions), [folders, sessions]);
  const rows = useMemo(() => flattenTree(index, expanded), [index, expanded]);
  // The input updates immediately; filtering runs at lower priority.
  const deferredQuery = useDeferredValue(query);
  const results = useMemo(
    () => (deferredQuery.trim() ? searchSessions(index, sessions, deferredQuery) : null),
    [index, sessions, deferredQuery],
  );
  const favorites = useMemo(() => sessions.filter((s) => s.favorite), [sessions]);
  const dnd = useTreeDrag(index);

  if (results) {
    const shown = results.slice(0, MAX_RESULTS);
    return (
      <TreeMenu index={index} actions={actions}>
        <div className="flex-1 overflow-y-auto py-1" role="tree" aria-label="Search results">
          {results.length === 0 ? (
            <p className="text-muted-foreground px-4 py-6 text-center text-sm">
              No sessions match “{deferredQuery.trim()}”.
            </p>
          ) : (
            shown.map((s) => (
              <SessionRow
                key={s.id}
                session={s}
                depth={0}
                subtitle={[folderPath(index, s.folderId), s.host].filter(Boolean).join(" · ")}
                actions={actions}
              />
            ))
          )}
          {results.length > shown.length && (
            <p className="text-muted-foreground px-4 py-3 text-center text-xs">
              Showing {shown.length} of {results.length} matches. Type more to narrow down.
            </p>
          )}
        </div>
      </TreeMenu>
    );
  }

  if (folders.length === 0 && sessions.length === 0) {
    return (
      <div className="text-muted-foreground flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center text-sm">
        <FolderClosed className="size-8 opacity-40" />
        <p>No saved sessions yet.</p>
        <button
          type="button"
          className="text-brand text-sm hover:underline"
          onClick={() => {
            actions.newSession(null);
          }}
        >
          Create your first session
        </button>
      </div>
    );
  }

  return (
    <TreeMenu index={index} actions={actions}>
      <div
        className={cn(
          "flex-1 overflow-y-auto py-1",
          dnd.target?.kind === "root" && "bg-brand/5 outline-brand/40 outline-1 -outline-offset-1",
        )}
        role="tree"
        aria-label="Saved sessions"
        data-drop="root"
      >
        {favorites.length > 0 && (
          <div className="mb-1">
            <div className="text-muted-foreground flex items-center gap-1.5 px-3 py-1 text-[11px] font-semibold tracking-wider uppercase">
              <Star className="size-3" /> Favourites
            </div>
            {favorites.map((s) => (
              <SessionRow key={`fav-${s.id}`} session={s} depth={0} actions={actions} />
            ))}
            <div className="border-sidebar-border mx-3 my-1 border-t" />
          </div>
        )}

        {rows.map((row) =>
          row.kind === "folder" ? (
            <FolderRow
              key={row.folder.id}
              folder={row.folder}
              depth={row.depth}
              expanded={row.expanded}
              count={row.count}
              renaming={renamingId === row.folder.id}
              onRenameDone={() => {
                setRenamingId(null);
              }}
              onStartRename={() => {
                setRenamingId(row.folder.id);
              }}
              dropTarget={dnd.target?.kind === "folder" && dnd.target.id === row.folder.id}
              dimmed={dnd.dragging?.id === row.folder.id}
              onPointerDown={dnd.start({ kind: "folder", id: row.folder.id })}
              consumeClick={dnd.consumeClick}
              actions={actions}
            />
          ) : (
            <SessionRow
              key={row.session.id}
              session={row.session}
              depth={row.depth}
              dropTarget={dnd.target?.kind === "session" && dnd.target.id === row.session.id}
              dimmed={dnd.dragging?.id === row.session.id}
              onPointerDown={dnd.start({ kind: "session", id: row.session.id })}
              consumeClick={dnd.consumeClick}
              draggable
              actions={actions}
            />
          ),
        )}
        {/* Spacer: dropping below the last row moves to the top level. */}
        <div className="h-8" data-drop="root" />
      </div>
    </TreeMenu>
  );
}

type MenuTarget =
  | { kind: "folder"; folder: Folder; count: number }
  | { kind: "session"; session: SavedSession }
  | { kind: "empty" };

/**
 * One context menu for the whole tree. Rendering a Radix menu per row made
 * typing in the search box lag with hundreds of sessions; instead the
 * right-clicked row is looked up from its `data-row` / `data-id` attributes.
 */
function TreeMenu({
  index,
  actions,
  children,
}: {
  index: LibraryIndex;
  actions: Actions;
  children: React.ReactElement;
}) {
  const sessions = useLibraryStore((s) => s.sessions);
  const newFolder = useLibraryStore((s) => s.newFolder);
  const setRenaming = useLibraryStore((s) => s.setRenamingFolder);
  const setFavorite = useLibraryStore((s) => s.setFavorite);
  const duplicateSession = useLibraryStore((s) => s.duplicateSession);
  const openFolderCredential = useDialogStore((s) => s.openFolderCredential);
  const [target, setTarget] = useState<MenuTarget>({ kind: "empty" });

  const onContextMenu = (e: React.MouseEvent) => {
    const row = (e.target as HTMLElement).closest<HTMLElement>("[data-row]");
    const id = row?.dataset.id;
    if (row?.dataset.row === "folder" && id) {
      const folder = index.folderById.get(id);
      if (folder) {
        setTarget({ kind: "folder", folder, count: countSessions(index, id) });
        return;
      }
    }
    if (row?.dataset.row === "session" && id) {
      const session = sessions.find((s) => s.id === id);
      if (session) {
        setTarget({ kind: "session", session });
        return;
      }
    }
    setTarget({ kind: "empty" });
  };

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild onContextMenu={onContextMenu}>
        {children}
      </ContextMenuTrigger>
      <ContextMenuContent>
        {target.kind === "session" && (
          <>
            <ContextMenuItem
              onSelect={() => {
                actions.connect(target.session);
              }}
            >
              <Plug /> Connect
            </ContextMenuItem>
            <ContextMenuItem
              onSelect={() => {
                actions.edit(target.session);
              }}
            >
              <Pencil /> Edit
            </ContextMenuItem>
            <ContextMenuItem onSelect={() => void duplicateSession(target.session.id)}>
              <Copy /> Duplicate
            </ContextMenuItem>
            <ContextMenuItem
              onSelect={() => void setFavorite(target.session.id, !target.session.favorite)}
            >
              {target.session.favorite ? <StarOff /> : <Star />}
              {target.session.favorite ? "Remove from favourites" : "Add to favourites"}
            </ContextMenuItem>
            <ContextMenuSeparator />
            <ContextMenuItem
              variant="destructive"
              onSelect={() => {
                actions.confirmDeleteSession(target.session);
              }}
            >
              <Trash2 /> Delete
            </ContextMenuItem>
          </>
        )}
        {target.kind === "folder" && (
          <>
            <ContextMenuItem
              onSelect={() => {
                actions.newSession(target.folder.id);
              }}
            >
              <Plus /> New session here
            </ContextMenuItem>
            <ContextMenuItem onSelect={() => void newFolder(target.folder.id)}>
              <FolderPlus /> New subfolder
            </ContextMenuItem>
            <ContextMenuItem
              onSelect={() => {
                openFolderCredential(target.folder.id);
              }}
            >
              <KeyRound /> Default credential…
            </ContextMenuItem>
            <ContextMenuSeparator />
            <ContextMenuItem
              onSelect={() => {
                setRenaming(target.folder.id);
              }}
            >
              <Pencil /> Rename
            </ContextMenuItem>
            <ContextMenuItem
              variant="destructive"
              onSelect={() => {
                actions.confirmDeleteFolder(target.folder, target.count);
              }}
            >
              <Trash2 /> Delete folder
            </ContextMenuItem>
          </>
        )}
        {target.kind === "empty" && (
          <>
            <ContextMenuItem
              onSelect={() => {
                actions.newSession(null);
              }}
            >
              <Plus /> New session
            </ContextMenuItem>
            <ContextMenuItem onSelect={() => void newFolder(null)}>
              <FolderPlus /> New folder
            </ContextMenuItem>
          </>
        )}
      </ContextMenuContent>
    </ContextMenu>
  );
}

type Actions = ReturnType<typeof useLibraryActions>;

interface FolderRowProps {
  folder: Folder;
  depth: number;
  expanded: boolean;
  count: number;
  renaming: boolean;
  onStartRename: () => void;
  onRenameDone: () => void;
  dropTarget: boolean;
  dimmed: boolean;
  onPointerDown: (e: React.PointerEvent) => void;
  consumeClick: () => boolean;
  actions: Actions;
}

function FolderRow({
  folder,
  depth,
  expanded,
  count,
  renaming,
  onStartRename,
  onRenameDone,
  dropTarget,
  dimmed,
  onPointerDown,
  consumeClick,
  actions,
}: FolderRowProps) {
  const toggleFolder = useLibraryStore((s) => s.toggleFolder);
  const renameFolder = useLibraryStore((s) => s.renameFolder);
  const Icon = expanded ? FolderOpen : FolderClosed;

  return (
    <div
      role="treeitem"
      aria-expanded={expanded}
      aria-selected={false}
      tabIndex={0}
      data-drop="folder"
      data-row="folder"
      data-id={folder.id}
      onPointerDown={renaming ? undefined : onPointerDown}
      onClick={() => {
        if (consumeClick() || renaming) return;
        toggleFolder(folder.id);
      }}
      onKeyDown={(e) => {
        if (renaming) return;
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          toggleFolder(folder.id);
        }
        if (e.key === "F2") onStartRename();
        if (e.key === "Delete") actions.confirmDeleteFolder(folder, count);
      }}
      style={{ paddingLeft: 8 + depth * INDENT_PX }}
      className={cn(
        "hover:bg-sidebar-accent focus-visible:bg-sidebar-accent flex h-7 cursor-default items-center gap-1.5 pr-2 text-sm outline-none select-none",
        dropTarget && "bg-brand/15 ring-brand ring-1 ring-inset",
        dimmed && "opacity-50",
      )}
    >
      <ChevronRight
        className={cn(
          "text-muted-foreground size-3.5 shrink-0 transition-transform",
          expanded && "rotate-90",
        )}
      />
      <Icon className="text-brand/80 size-4 shrink-0" />
      {renaming ? (
        <InlineInput
          initial={folder.name}
          onDone={(value) => {
            onRenameDone();
            if (value && value !== folder.name) void renameFolder(folder.id, value);
          }}
        />
      ) : (
        <span className="min-w-0 flex-1 truncate">{folder.name}</span>
      )}
      {!renaming && <span className="text-muted-foreground text-xs">{count}</span>}
    </div>
  );
}

interface SessionRowProps {
  session: SavedSession;
  depth: number;
  subtitle?: string;
  dropTarget?: boolean;
  dimmed?: boolean;
  draggable?: boolean;
  onPointerDown?: (e: React.PointerEvent) => void;
  consumeClick?: () => boolean;
  actions: Actions;
}

function SessionRow({
  session,
  depth,
  subtitle,
  dropTarget,
  dimmed,
  draggable,
  onPointerDown,
  consumeClick,
  actions,
}: SessionRowProps) {
  const target = `${session.username ? `${session.username}@` : ""}${session.host}${session.port === 22 ? "" : `:${session.port}`}`;

  return (
    <div
      role="treeitem"
      aria-selected={false}
      tabIndex={0}
      title={[target, ...session.tags.map((t) => `#${t}`), session.notes]
        .filter(Boolean)
        .join("\n")}
      data-drop={draggable ? "session" : undefined}
      data-row="session"
      data-id={session.id}
      onPointerDown={onPointerDown}
      onClick={() => {
        consumeClick?.();
      }}
      onDoubleClick={() => {
        actions.connect(session);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") actions.connect(session);
        if (e.key === "Delete") actions.confirmDeleteSession(session);
      }}
      style={{ paddingLeft: 8 + depth * INDENT_PX + (draggable ? 20 : 4) }}
      className={cn(
        "hover:bg-sidebar-accent focus-visible:bg-sidebar-accent flex cursor-default items-center gap-2 pr-2 text-sm outline-none select-none",
        subtitle ? "py-1" : "h-7",
        dropTarget && "shadow-[inset_0_2px_0_var(--color-brand)]",
        dimmed && "opacity-50",
      )}
    >
      <span
        className="size-2 shrink-0 rounded-full"
        style={{ backgroundColor: session.color ?? "var(--color-muted-foreground)" }}
        aria-hidden
      />
      <span className="min-w-0 flex-1">
        <span className="block truncate">{session.name}</span>
        {subtitle && (
          <span className="text-muted-foreground block truncate text-xs">{subtitle}</span>
        )}
      </span>
      {session.favorite && !subtitle && (
        <Star className="size-3 shrink-0 fill-amber-400 text-amber-400" aria-label="Favourite" />
      )}
    </div>
  );
}

/** Inline name editor. Enter/blur saves, Escape cancels. */
function InlineInput({
  initial,
  onDone,
}: {
  initial: string;
  onDone: (value: string | null) => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  const done = useRef(false);
  useEffect(() => {
    ref.current?.select();
  }, []);
  const finish = (value: string | null) => {
    if (done.current) return;
    done.current = true;
    onDone(value?.trim() ? value.trim() : null);
  };
  return (
    <input
      ref={ref}
      defaultValue={initial}
      maxLength={64}
      aria-label="Folder name"
      className="bg-background min-w-0 flex-1 rounded border px-1 text-sm outline-none"
      onClick={(e) => {
        e.stopPropagation();
      }}
      onPointerDown={(e) => {
        e.stopPropagation();
      }}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") finish(e.currentTarget.value);
        if (e.key === "Escape") finish(null);
      }}
      onBlur={(e) => {
        finish(e.currentTarget.value);
      }}
    />
  );
}
