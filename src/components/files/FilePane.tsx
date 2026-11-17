import { useMemo, useRef, useState, type SyntheticEvent } from "react";
import {
  ArrowLeftRight,
  ArrowUp,
  ChevronDown,
  ChevronRight,
  ClipboardCopy,
  Download,
  Eye,
  EyeOff,
  FilePen,
  FolderOpen,
  FolderPlus,
  HardDrive,
  Home,
  KeyRound,
  Loader2,
  Lock,
  Maximize2,
  Pencil,
  RefreshCw,
  Search,
  Server,
  ShieldAlert,
  Trash2,
  Unplug,
  Upload,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { Input } from "@/components/ui/input";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import {
  changePermissions,
  copyPathToClipboard,
  copyToOtherPane,
  deleteItems,
  downloadTo,
  editItem,
  newFolder,
  renameItem,
  uploadFiles,
} from "@/lib/fileActions";
import { FileList } from "@/components/files/FileList";
import { LocationPicker } from "@/components/files/LocationPicker";
import {
  breadcrumbs,
  clickSelection,
  formatBytes,
  sortEntries,
  type SortKey,
  type SortSpec,
} from "@/lib/files";
import { cn } from "@/lib/utils";
import { isLocal, useFilesStore, type Pane, type PaneIndex } from "@/stores/files";
import { isDirLike } from "@/types/files";

interface FilePaneProps {
  tabId: string;
  index: PaneIndex;
  pane: Pane;
  active: boolean;
  /** "side" = the SFTP panel next to a terminal: Upload…/Download… instead of two panes. */
  mode?: "tab" | "side";
  /** Side panel: open this server in a full file-manager tab. */
  onOpenFull?: () => void;
  /** Folder (or the pane itself: "") the user is dragging over, if this pane. */
  dropDir: string | null;
  onDragStart: (index: PaneIndex, paths: string[], e: React.PointerEvent) => void;
}

export function FilePane({
  tabId,
  index,
  pane,
  active,
  mode = "tab",
  onOpenFull,
  dropDir,
  onDragStart,
}: FilePaneProps) {
  const side = mode === "side";
  const navigate = useFilesStore((s) => s.navigate);
  const refresh = useFilesStore((s) => s.refresh);
  const setSelected = useFilesStore((s) => s.setSelected);
  const setActivePane = useFilesStore((s) => s.setActivePane);
  const toggleHidden = useFilesStore((s) => s.toggleHidden);
  const setSource = useFilesStore((s) => s.setSource);
  const setNotice = useFilesStore((s) => s.setNotice);

  const [sort, setSort] = useState<SortSpec>({ key: "name", asc: true });
  const [filter, setFilter] = useState("");
  const [picking, setPicking] = useState(false);
  const [cursor, setCursor] = useState<number | null>(null);
  const anchor = useRef<number | null>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const local = isLocal(pane);
  const entries = useMemo(() => {
    const all = pane.listing?.entries ?? [];
    const q = filter.trim().toLowerCase();
    const shown = all.filter(
      (e) => (pane.showHidden || !e.hidden) && (!q || e.name.toLowerCase().includes(q)),
    );
    return sortEntries(shown, sort);
  }, [pane.listing, pane.showHidden, filter, sort]);
  const paths = useMemo(() => entries.map((e) => e.path), [entries]);
  const selected = useMemo(() => new Set(pane.selected), [pane.selected]);
  const selectedEntries = entries.filter((e) => selected.has(e.path));
  const selectedBytes = selectedEntries.reduce((n, e) => n + (isDirLike(e) ? 0 : e.size), 0);

  const ready = pane.status === "ready";
  const atDriveList = local && pane.path === "";
  const select = (next: string[]) => {
    setSelected(tabId, index, next);
  };
  const go = async (path: string) => {
    setFilter("");
    setCursor(null);
    anchor.current = null;
    if (await navigate(tabId, index, path)) listRef.current?.focus();
  };
  const open = (i: number) => {
    const e = entries[i];
    if (!e) return;
    if (isDirLike(e)) void go(e.path);
    else if (!local && e.kind === "file") editItem(tabId, index, e.path);
  };
  const goUp = () => {
    if (pane.listing?.parent != null) void go(pane.listing.parent);
  };
  const onSort = (key: SortKey) => {
    setSort((s) => (s.key === key ? { key, asc: !s.asc } : { key, asc: true }));
  };
  const targets = () => (pane.selected.length > 0 ? pane.selected : []);

  function onKeyDown(e: React.KeyboardEvent) {
    if (!ready) return;
    const ctrl = e.ctrlKey || e.metaKey;
    const move = (to: number) => {
      const i = Math.max(0, Math.min(entries.length - 1, to));
      setCursor(i);
      if (e.shiftKey && anchor.current !== null) {
        const r = clickSelection(paths, selected, anchor.current, i, { ctrl: false, shift: true });
        select([...r.selected]);
      } else if (!ctrl) {
        anchor.current = i;
        const p = paths[i];
        if (p !== undefined) select([p]);
      }
    };
    const page = Math.max(1, Math.floor((listRef.current?.clientHeight ?? 260) / 26) - 1);
    const handled = (() => {
      switch (e.key) {
        case "ArrowDown":
          move((cursor ?? -1) + 1);
          return true;
        case "ArrowUp":
          move((cursor ?? entries.length) - 1);
          return true;
        case "PageDown":
          move((cursor ?? 0) + page);
          return true;
        case "PageUp":
          move((cursor ?? 0) - page);
          return true;
        case "Home":
          move(0);
          return true;
        case "End":
          move(entries.length - 1);
          return true;
        case "Enter":
          if (cursor !== null) open(cursor);
          return true;
        case "Backspace":
          goUp();
          return true;
        case "Delete":
          if (targets().length) deleteItems(tabId, index, targets());
          return true;
        case "F2": {
          const t = targets();
          if (t.length === 1 && t[0]) renameItem(tabId, index, t[0]);
          return true;
        }
        case "F4": {
          const t = targets();
          if (!local && t.length === 1 && t[0]) editItem(tabId, index, t[0]);
          return true;
        }
        case "F5":
          if (targets().length) {
            if (side) void downloadTo(tabId, index, targets());
            else void copyToOtherPane(tabId, index, targets());
          }
          return true;
        case "F7":
          newFolder(tabId, index);
          return true;
        case " ":
          if (cursor !== null) {
            const r = clickSelection(paths, selected, anchor.current, cursor, {
              ctrl: true,
              shift: false,
            });
            select([...r.selected]);
          }
          return true;
        default:
          if (ctrl && e.key.toLowerCase() === "a") {
            select(paths);
            return true;
          }
          if (ctrl && e.key.toLowerCase() === "r") {
            void refresh(tabId, index);
            return true;
          }
          return false;
      }
    })();
    if (handled) e.preventDefault();
  }

  return (
    <section
      aria-label={`${index === 0 ? "Left" : "Right"} pane: ${pane.label}`}
      data-pane-index={index}
      className={cn(
        "bg-background flex min-h-0 min-w-0 flex-1 flex-col",
        dropDir === "" && "ring-brand ring-2 ring-inset",
      )}
      onPointerDownCapture={() => {
        if (!active) setActivePane(tabId, index);
      }}
    >
      {/* Location + actions */}
      <div
        className={cn(
          "flex h-9 shrink-0 items-center gap-1 border-b px-2",
          active ? "bg-sidebar-accent/60" : "bg-sidebar",
        )}
      >
        <button
          type="button"
          disabled={side}
          className="hover:bg-accent flex min-w-0 items-center gap-2 rounded px-2 py-1 text-sm font-medium disabled:hover:bg-transparent"
          onClick={() => {
            setPicking((p) => !p);
          }}
          aria-expanded={picking || pane.status === "picker"}
          title={side ? pane.label : "Change location"}
        >
          {pane.source?.kind === "local" ? (
            <HardDrive className="text-brand size-4 shrink-0" />
          ) : (
            <Server className="text-brand size-4 shrink-0" />
          )}
          <span className="truncate">{pane.label}</span>
          {!side && <ChevronDown className="text-muted-foreground size-3.5 shrink-0" />}
        </button>
        <ProtocolBadge pane={pane} />
        <div className="flex-1" />
        {side && (
          <IconButton
            label="Upload files…"
            disabled={!ready}
            onClick={() => void uploadFiles(tabId, index)}
          >
            <Upload />
          </IconButton>
        )}
        <IconButton
          label={pane.showHidden ? "Hide hidden files" : "Show hidden files"}
          disabled={!ready}
          onClick={() => {
            toggleHidden(tabId, index);
          }}
        >
          {pane.showHidden ? <Eye /> : <EyeOff />}
        </IconButton>
        <IconButton
          label="New folder (F7)"
          disabled={!ready || atDriveList}
          onClick={() => {
            newFolder(tabId, index);
          }}
        >
          <FolderPlus />
        </IconButton>
        <IconButton
          label="Refresh (Ctrl+R)"
          disabled={!ready}
          onClick={() => void refresh(tabId, index)}
        >
          <RefreshCw className={cn(pane.loading && "animate-spin")} />
        </IconButton>
        {side && onOpenFull && (
          <IconButton label="Open in file manager tab" onClick={onOpenFull}>
            <Maximize2 />
          </IconButton>
        )}
        {!side && !local && pane.status !== "picker" && (
          <IconButton label="Disconnect" onClick={() => void setSource(tabId, index, null)}>
            <Unplug />
          </IconButton>
        )}
      </div>

      {/* Path */}
      <div className="flex h-9 shrink-0 items-center gap-1 border-b px-2">
        <IconButton
          label="Up (Backspace)"
          disabled={!ready || pane.listing?.parent == null}
          onClick={goUp}
        >
          <ArrowUp />
        </IconButton>
        <IconButton label="Home" disabled={!ready} onClick={() => void go(pane.home)}>
          <Home />
        </IconButton>
        <PathBar path={pane.path} local={local} disabled={!ready} onNavigate={(p) => void go(p)} />
        <div className={cn("relative shrink-0", side ? "w-24" : "w-36")}>
          <Search className="text-muted-foreground absolute top-1/2 left-2 size-3.5 -translate-y-1/2" />
          <Input
            value={filter}
            disabled={!ready}
            placeholder="Filter"
            aria-label="Filter files in this folder"
            className="h-7 pl-7 text-xs"
            onChange={(e) => {
              setFilter(e.target.value);
            }}
            onKeyDown={(e) => {
              if (e.key === "Escape") setFilter("");
              if (e.key === "ArrowDown") listRef.current?.focus();
            }}
          />
        </div>
      </div>

      {pane.notice && (
        <div
          role="status"
          className="flex items-center gap-2 border-b bg-amber-500/10 px-3 py-1.5 text-xs text-amber-200"
        >
          <span className="min-w-0 flex-1 break-words">{pane.notice}</span>
          <button
            type="button"
            className="hover:text-foreground"
            onClick={() => {
              setNotice(tabId, index, null);
            }}
          >
            Dismiss
          </button>
        </div>
      )}

      {/* Body */}
      {picking || pane.status === "picker" ? (
        <LocationPicker
          onPick={(source) => {
            setPicking(false);
            void setSource(tabId, index, source);
          }}
          onCancel={
            pane.status === "picker"
              ? undefined
              : () => {
                  setPicking(false);
                }
          }
        />
      ) : pane.status === "connecting" ? (
        <CenterMessage>
          <Loader2 className="text-brand size-6 animate-spin" />
          <span>Connecting to {pane.label}…</span>
        </CenterMessage>
      ) : pane.status === "needsLogin" ? (
        <PaneLogin
          reason={pane.error ?? "Enter your login."}
          onSubmit={(login) =>
            void setSource(tabId, index, pane.source, login, pane.path || undefined)
          }
          onCancel={() => void setSource(tabId, index, null)}
        />
      ) : pane.status === "error" ? (
        <CenterMessage>
          <ShieldAlert className="text-destructive size-6" />
          <span className="max-w-sm text-center break-words">{pane.error}</span>
          <div className="flex gap-2">
            <Button
              size="sm"
              onClick={() => void setSource(tabId, index, pane.source, undefined, pane.path)}
            >
              <RefreshCw /> Reconnect
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() => {
                setPicking(true);
              }}
            >
              Choose another
            </Button>
          </div>
        </CenterMessage>
      ) : (
        <ContextMenu>
          <ContextMenuTrigger asChild>
            <div
              ref={listRef}
              tabIndex={0}
              aria-label="Files"
              className="flex min-h-0 flex-1 flex-col outline-none"
              onKeyDown={onKeyDown}
              onFocus={() => {
                if (!active) setActivePane(tabId, index);
              }}
            >
              {entries.length === 0 && !pane.loading ? (
                <CenterMessage>
                  <FolderOpen className="text-muted-foreground size-6" />
                  <span>{filter ? "No matching files." : "This folder is empty."}</span>
                </CenterMessage>
              ) : (
                <FileList
                  entries={entries}
                  selected={selected}
                  cursor={cursor}
                  sort={sort}
                  remote={!local}
                  focused={active}
                  dropDir={dropDir}
                  onSort={onSort}
                  onRowPointerDown={(i, e) => {
                    if (e.button !== 0) return;
                    const p = paths[i];
                    if (p === undefined) return;
                    // Dragging an unselected row drags just that row.
                    const dragged = selected.has(p) ? pane.selected : [p];
                    onDragStart(index, dragged, e);
                  }}
                  onRowClick={(i, e) => {
                    const r = clickSelection(paths, selected, anchor.current, i, {
                      ctrl: e.ctrlKey || e.metaKey,
                      shift: e.shiftKey,
                    });
                    anchor.current = r.anchor;
                    setCursor(i);
                    select([...r.selected]);
                  }}
                  onRowDoubleClick={open}
                  onRowContextMenu={(i) => {
                    const p = paths[i];
                    setCursor(i);
                    if (p !== undefined && !selected.has(p)) {
                      anchor.current = i;
                      select([p]);
                    }
                  }}
                  onBackgroundClick={() => {
                    select([]);
                  }}
                />
              )}
            </div>
          </ContextMenuTrigger>
          <PaneMenu
            tabId={tabId}
            index={index}
            pane={pane}
            side={side}
            onOpen={open}
            cursor={cursor}
          />
        </ContextMenu>
      )}

      {/* Status line */}
      <div className="text-muted-foreground flex h-6 shrink-0 items-center gap-3 border-t px-3 text-xs">
        {ready && pane.listing && (
          <>
            <span>
              {entries.length} {entries.length === 1 ? "item" : "items"}
            </span>
            {pane.selected.length > 0 && (
              <span>
                {pane.selected.length} selected
                {selectedBytes > 0 && ` (${formatBytes(selectedBytes)})`}
              </span>
            )}
          </>
        )}
        {pane.loading && <Loader2 className="size-3 animate-spin" />}
      </div>
    </section>
  );
}

function PaneMenu({
  tabId,
  index,
  pane,
  side,
  onOpen,
  cursor,
}: {
  tabId: string;
  index: PaneIndex;
  pane: Pane;
  side: boolean;
  onOpen: (i: number) => void;
  cursor: number | null;
}) {
  const sel = pane.selected;
  const single =
    sel.length === 1 ? pane.listing?.entries.find((e) => e.path === sel[0]) : undefined;
  const local = isLocal(pane);
  const refresh = useFilesStore((s) => s.refresh);
  const noFolder = local && pane.path === "";
  return (
    <ContextMenuContent className="w-56">
      {single && isDirLike(single) && (
        <ContextMenuItem
          onSelect={() => {
            if (cursor !== null) onOpen(cursor);
          }}
        >
          <FolderOpen /> Open
        </ContextMenuItem>
      )}
      {!local && single && single.kind === "file" && (
        <ContextMenuItem
          onSelect={() => {
            editItem(tabId, index, single.path);
          }}
        >
          <FilePen /> Edit in Notepad
          <ContextMenuShortcut>F4</ContextMenuShortcut>
        </ContextMenuItem>
      )}
      {side ? (
        <ContextMenuItem
          disabled={sel.length === 0}
          onSelect={() => void downloadTo(tabId, index, sel)}
        >
          <Download /> Download…
          <ContextMenuShortcut>F5</ContextMenuShortcut>
        </ContextMenuItem>
      ) : (
        <ContextMenuItem
          disabled={sel.length === 0}
          onSelect={() => void copyToOtherPane(tabId, index, sel)}
        >
          <ArrowLeftRight /> Copy to other side
          <ContextMenuShortcut>F5</ContextMenuShortcut>
        </ContextMenuItem>
      )}
      <ContextMenuSeparator />
      <ContextMenuItem
        disabled={!single || noFolder}
        onSelect={() => {
          if (single) renameItem(tabId, index, single.path);
        }}
      >
        <Pencil /> Rename
        <ContextMenuShortcut>F2</ContextMenuShortcut>
      </ContextMenuItem>
      {!local && (
        <ContextMenuItem
          disabled={sel.length === 0}
          onSelect={() => {
            changePermissions(tabId, index, sel);
          }}
        >
          <KeyRound /> Permissions…
        </ContextMenuItem>
      )}
      <ContextMenuItem disabled={sel.length === 0} onSelect={() => void copyPathToClipboard(sel)}>
        <ClipboardCopy /> Copy path
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem
        disabled={noFolder}
        onSelect={() => {
          newFolder(tabId, index);
        }}
      >
        <FolderPlus /> New folder
        <ContextMenuShortcut>F7</ContextMenuShortcut>
      </ContextMenuItem>
      <ContextMenuItem onSelect={() => void refresh(tabId, index)}>
        <RefreshCw /> Refresh
        <ContextMenuShortcut>Ctrl+R</ContextMenuShortcut>
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem
        variant="destructive"
        disabled={sel.length === 0 || noFolder}
        onSelect={() => {
          deleteItems(tabId, index, sel);
        }}
      >
        <Trash2 /> Delete
        <ContextMenuShortcut>Del</ContextMenuShortcut>
      </ContextMenuItem>
    </ContextMenuContent>
  );
}

/** "Unencrypted" warning for plain FTP; a small FTPS tag otherwise. */
function ProtocolBadge({ pane }: { pane: Pane }) {
  if (pane.status !== "ready") return null;
  if (pane.protocol === "ftp") {
    return (
      <span
        className="flex shrink-0 items-center gap-1 rounded border border-amber-500/50 bg-amber-500/15 px-1.5 py-0.5 text-[11px] font-medium text-amber-300"
        title="Plain FTP sends your password and files unencrypted. Use SFTP or FTPS where possible."
      >
        <ShieldAlert className="size-3" /> Unencrypted
      </span>
    );
  }
  if (pane.protocol === "ftps") {
    return (
      <span
        className="text-muted-foreground flex shrink-0 items-center gap-1 rounded border px-1.5 py-0.5 text-[11px]"
        title="FTP over TLS"
      >
        <Lock className="size-3" /> FTPS
      </span>
    );
  }
  return null;
}

/** Breadcrumbs; click the empty space (or press Ctrl+L) to type a path. */
function PathBar({
  path,
  local,
  disabled,
  onNavigate,
}: {
  path: string;
  local: boolean;
  disabled: boolean;
  onNavigate: (path: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(path);
  const crumbs = breadcrumbs(path, local);

  if (editing) {
    const submit = (e: SyntheticEvent) => {
      e.preventDefault();
      setEditing(false);
      if (draft.trim() && draft !== path) onNavigate(draft.trim());
    };
    return (
      <form onSubmit={submit} className="min-w-0 flex-1">
        <Input
          autoFocus
          value={draft}
          spellCheck={false}
          aria-label="Path"
          className="h-7 font-mono text-xs"
          onChange={(e) => {
            setDraft(e.target.value);
          }}
          onBlur={() => {
            setEditing(false);
          }}
          onKeyDown={(e) => {
            if (e.key === "Escape") setEditing(false);
          }}
        />
      </form>
    );
  }
  return (
    <div
      role="navigation"
      aria-label="Path"
      className={cn(
        "hover:border-input flex h-7 min-w-0 flex-1 items-center overflow-hidden rounded-md border border-transparent px-1 text-xs",
        disabled && "opacity-50",
      )}
      onClick={(e) => {
        if (disabled || e.target !== e.currentTarget) return;
        setDraft(path);
        setEditing(true);
      }}
      title="Click empty space to type a path"
    >
      {crumbs.length === 0 && <span className="text-muted-foreground px-1">This PC</span>}
      {crumbs.map((c, i) => (
        <span
          key={c.path}
          className={cn("flex min-w-0 items-center", i < crumbs.length - 2 && "shrink-[3]")}
        >
          {i > 0 && <ChevronRight className="text-muted-foreground size-3 shrink-0" />}
          <button
            type="button"
            disabled={disabled}
            className={cn(
              "hover:bg-accent truncate rounded px-1 py-0.5",
              i === crumbs.length - 1 && "text-foreground font-medium",
            )}
            onClick={() => {
              onNavigate(c.path);
            }}
          >
            {c.label}
          </button>
        </span>
      ))}
      <span
        className="min-w-4 flex-1 self-stretch"
        onClick={() => {
          if (disabled) return;
          setDraft(path);
          setEditing(true);
        }}
      />
    </div>
  );
}

function PaneLogin({
  reason,
  onSubmit,
  onCancel,
}: {
  reason: string;
  onSubmit: (login: { username?: string; password?: string }) => void;
  onCancel: () => void;
}) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  return (
    <form
      className="m-auto grid w-72 gap-3 p-4"
      onSubmit={(e) => {
        e.preventDefault();
        const login = { username: username.trim() || undefined, password: password || undefined };
        setPassword("");
        onSubmit(login);
      }}
    >
      <p className="text-sm">{reason}</p>
      <Input
        placeholder="Username (if not saved)"
        aria-label="Username"
        value={username}
        autoComplete="off"
        onChange={(e) => {
          setUsername(e.target.value);
        }}
      />
      <Input
        type="password"
        placeholder="Password"
        aria-label="Password"
        autoFocus
        value={password}
        autoComplete="off"
        onChange={(e) => {
          setPassword(e.target.value);
        }}
      />
      <div className="flex justify-end gap-2">
        <Button type="button" variant="outline" size="sm" onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" size="sm">
          Connect
        </Button>
      </div>
    </form>
  );
}

function CenterMessage({ children }: { children: React.ReactNode }) {
  return (
    <div className="text-muted-foreground flex flex-1 flex-col items-center justify-center gap-3 p-6 text-sm">
      {children}
    </div>
  );
}

function IconButton({
  label,
  children,
  disabled,
  onClick,
}: {
  label: string;
  children: React.ReactNode;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={label}
          disabled={disabled}
          onClick={onClick}
          className="shrink-0"
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}
