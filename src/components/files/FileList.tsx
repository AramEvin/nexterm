import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { ArrowDown, ArrowUp, File, FileSymlink, Folder, FolderSymlink } from "lucide-react";

import { formatBytes, formatDate, formatMode, type SortKey, type SortSpec } from "@/lib/files";
import { cn } from "@/lib/utils";
import { isDirLike, type FileEntry } from "@/types/files";

/** Fixed row height: lets us render only the visible rows (10k+ folders stay fast). */
export const ROW_HEIGHT = 26;
const OVERSCAN = 8;

interface Column {
  key: SortKey;
  label: string;
  width: string;
  align?: "right";
}

interface FileListProps {
  entries: FileEntry[];
  selected: ReadonlySet<string>;
  /** Keyboard cursor (index into `entries`). */
  cursor: number | null;
  sort: SortSpec;
  remote: boolean;
  focused: boolean;
  /** Folder row the user is dragging over (highlighted). */
  dropDir: string | null;
  onSort: (key: SortKey) => void;
  onRowPointerDown: (index: number, e: React.PointerEvent) => void;
  onRowClick: (index: number, e: React.MouseEvent) => void;
  onRowDoubleClick: (index: number) => void;
  onRowContextMenu: (index: number) => void;
  onBackgroundClick: () => void;
}

export function FileList({
  entries,
  selected,
  cursor,
  sort,
  remote,
  focused,
  dropDir,
  onSort,
  onRowPointerDown,
  onRowClick,
  onRowDoubleClick,
  onRowContextMenu,
  onBackgroundClick,
}: FileListProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(400);
  const [width, setWidth] = useState(800);

  // Narrow panes drop the least important columns instead of scrolling sideways.
  const showPerms = remote && width >= 520;
  const showOwner = remote && width >= 620;
  const columns: Column[] = useMemo(
    () => [
      { key: "name", label: "Name", width: "minmax(8rem,1fr)" },
      { key: "size", label: "Size", width: "4.75rem", align: "right" },
      { key: "modified", label: "Modified", width: "8.25rem" },
      ...(showPerms
        ? ([{ key: "permissions", label: "Permissions", width: "6rem" }] satisfies Column[])
        : []),
      ...(showOwner ? ([{ key: "owner", label: "Owner", width: "5rem" }] satisfies Column[]) : []),
    ],
    [showPerms, showOwner],
  );
  const grid = columns.map((c) => c.width).join(" ");

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => {
      setHeight(el.clientHeight);
      setWidth(el.clientWidth);
    });
    ro.observe(el);
    setHeight(el.clientHeight);
    setWidth(el.clientWidth);
    return () => {
      ro.disconnect();
    };
  }, []);

  // Keep the keyboard cursor visible.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el || cursor === null) return;
    const top = cursor * ROW_HEIGHT;
    if (top < el.scrollTop) el.scrollTop = top;
    else if (top + ROW_HEIGHT > el.scrollTop + el.clientHeight) {
      el.scrollTop = top + ROW_HEIGHT - el.clientHeight;
    }
  }, [cursor]);

  const first = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const last = Math.min(entries.length, Math.ceil((scrollTop + height) / ROW_HEIGHT) + OVERSCAN);
  const visible = entries.slice(first, last);

  return (
    <div className="flex min-h-0 flex-1 flex-col text-[13px]">
      <div
        role="row"
        className="text-muted-foreground border-b px-2 text-xs select-none"
        style={{ display: "grid", gridTemplateColumns: grid }}
      >
        {columns.map((c) => (
          <button
            key={c.key}
            type="button"
            role="columnheader"
            aria-sort={sort.key === c.key ? (sort.asc ? "ascending" : "descending") : "none"}
            className={cn(
              "hover:text-foreground flex h-7 items-center gap-1 truncate px-1",
              c.align === "right" && "justify-end",
            )}
            onClick={() => {
              onSort(c.key);
            }}
          >
            {c.label}
            {sort.key === c.key &&
              (sort.asc ? <ArrowUp className="size-3" /> : <ArrowDown className="size-3" />)}
          </button>
        ))}
      </div>
      <div
        ref={scrollRef}
        role="grid"
        aria-multiselectable
        aria-rowcount={entries.length}
        className="relative min-h-0 flex-1 overflow-x-hidden overflow-y-auto"
        onScroll={(e) => {
          setScrollTop(e.currentTarget.scrollTop);
        }}
        onClick={(e) => {
          if (e.target === e.currentTarget) onBackgroundClick();
        }}
      >
        <div style={{ height: entries.length * ROW_HEIGHT }} className="relative">
          {visible.map((entry, i) => {
            const index = first + i;
            const isSelected = selected.has(entry.path);
            const dir = isDirLike(entry);
            return (
              <div
                key={entry.path}
                role="row"
                aria-selected={isSelected}
                aria-rowindex={index + 1}
                data-entry-path={entry.path}
                data-dir-path={dir ? entry.path : undefined}
                className={cn(
                  "absolute inset-x-0 cursor-default items-center px-2 select-none",
                  isSelected ? (focused ? "bg-brand/25" : "bg-accent") : "hover:bg-accent/50",
                  cursor === index && focused && "outline-ring outline-1 -outline-offset-1",
                  dropDir === entry.path && "ring-brand ring-2 ring-inset",
                  entry.hidden && "opacity-60",
                )}
                style={{
                  top: index * ROW_HEIGHT,
                  height: ROW_HEIGHT,
                  display: "grid",
                  gridTemplateColumns: grid,
                }}
                onPointerDown={(e) => {
                  onRowPointerDown(index, e);
                }}
                onClick={(e) => {
                  onRowClick(index, e);
                }}
                onDoubleClick={() => {
                  onRowDoubleClick(index);
                }}
                onContextMenu={() => {
                  onRowContextMenu(index);
                }}
              >
                <span className="flex min-w-0 items-center gap-2 px-1">
                  <EntryIcon entry={entry} />
                  <span className="truncate" title={entry.name}>
                    {entry.name}
                  </span>
                </span>
                <span className="text-muted-foreground truncate px-1 text-right tabular-nums">
                  {dir ? "" : formatBytes(entry.size)}
                </span>
                <span className="text-muted-foreground truncate px-1 tabular-nums">
                  {formatDate(entry.modified)}
                </span>
                {showPerms && (
                  <span
                    className="text-muted-foreground truncate px-1 font-mono text-xs"
                    title={entry.group ? `group ${entry.group}` : undefined}
                  >
                    {formatMode(entry.permissions)}
                  </span>
                )}
                {showOwner && (
                  <span className="text-muted-foreground truncate px-1">{entry.owner ?? ""}</span>
                )}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function EntryIcon({ entry }: { entry: FileEntry }) {
  const cls = "size-4 shrink-0";
  if (entry.kind === "symlink") {
    return entry.linkToDir ? (
      <FolderSymlink className={cn(cls, "text-sky-400")} aria-label="Link to folder" />
    ) : (
      <FileSymlink className={cn(cls, "text-muted-foreground")} aria-label="Link" />
    );
  }
  if (entry.kind === "dir")
    return <Folder className={cn(cls, "text-sky-400")} aria-label="Folder" />;
  return <File className={cn(cls, "text-muted-foreground")} aria-label="File" />;
}
