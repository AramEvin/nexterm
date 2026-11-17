import { useState } from "react";
import {
  ArrowRightLeft,
  Ban,
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Clock,
  FilePen,
  RotateCw,
  X,
  XCircle,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { formatBytes, formatEta, formatSpeed } from "@/lib/files";
import { cn } from "@/lib/utils";
import { useEditStore, type EditItem } from "@/stores/edits";
import { isFinished, useTransferStore, type TransferItem } from "@/stores/transfers";

/** Collapsible drawer under the panes: every copy with progress, cancel, retry. */
export function TransferQueue() {
  const items = useTransferStore((s) => s.items);
  const clearFinished = useTransferStore((s) => s.clearFinished);
  const edits = useEditStore((s) => s.items);
  const [open, setOpen] = useState(true);

  const active = items.filter((t) => !isFinished(t.status));
  const failed = items.filter((t) => t.status === "failed").length;
  const totalBytes = active.reduce((n, t) => n + t.bytesTotal, 0);
  const doneBytes = active.reduce((n, t) => n + t.bytesDone, 0);
  const speed = active.reduce((n, t) => n + t.bytesPerSec, 0);

  if (items.length === 0 && edits.length === 0) return null;
  const editing = edits.filter((e) => e.status !== "closed").length;

  return (
    <div className="bg-sidebar flex max-h-[32%] shrink-0 flex-col border-t">
      <button
        type="button"
        className="hover:bg-accent/50 flex h-8 shrink-0 items-center gap-3 px-3 text-left text-xs"
        aria-expanded={open}
        onClick={() => {
          setOpen((o) => !o);
        }}
      >
        <ArrowRightLeft className="text-brand size-3.5" />
        <span className="font-medium">Transfers</span>
        <span className="text-muted-foreground">
          {editing > 0 && `${String(editing)} editing · `}
          {active.length > 0
            ? `${String(active.length)} active · ${formatBytes(doneBytes)} of ${formatBytes(totalBytes)}${speed > 0 ? ` · ${formatSpeed(speed)}` : ""}`
            : `${String(items.length)} finished`}
          {failed > 0 && <span className="text-destructive"> · {failed} failed</span>}
        </span>
        <span className="flex-1" />
        {items.some((t) => isFinished(t.status)) && (
          <span
            role="button"
            tabIndex={0}
            className="text-muted-foreground hover:text-foreground rounded px-1"
            onClick={(e) => {
              e.stopPropagation();
              clearFinished();
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.stopPropagation();
                clearFinished();
              }
            }}
          >
            Clear finished
          </span>
        )}
        {open ? <ChevronDown className="size-3.5" /> : <ChevronUp className="size-3.5" />}
      </button>
      {open && (
        <ul className="min-h-0 overflow-auto border-t" aria-label="Transfers">
          {edits.map((e) => (
            <EditRow key={e.id} item={e} />
          ))}
          {[...items].reverse().map((t) => (
            <TransferRow key={t.id} item={t} />
          ))}
        </ul>
      )}
    </div>
  );
}

const timeFormat = new Intl.DateTimeFormat(undefined, {
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
});

/** A remote file open in Notepad: saves are uploaded; Stop deletes the local copy. */
function EditRow({ item }: { item: EditItem }) {
  const stop = useEditStore((s) => s.stop);
  const remove = useEditStore((s) => s.remove);
  const closed = item.status === "closed";
  return (
    <li className="flex items-center gap-3 border-b px-3 py-1.5 text-xs last:border-b-0">
      <FilePen
        className={cn("size-4 shrink-0", closed ? "text-muted-foreground" : "text-sky-400")}
        aria-label="Editing"
      />
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="truncate font-medium" title={item.remotePath}>
            {item.name}
          </span>
          <span className="text-muted-foreground truncate">{item.where}</span>
        </div>
        <div className="text-muted-foreground mt-0.5 truncate">
          {item.status === "opening" && "Downloading and opening the editor…"}
          {item.status === "editing" &&
            (item.lastSavedAt
              ? `Editing in ${item.editor} · saved to the server at ${timeFormat.format(new Date(item.lastSavedAt))}`
              : `Editing in ${item.editor} · each save is uploaded to the server`)}
          {closed && `Closed${item.saves > 0 ? ` · ${String(item.saves)} save(s) uploaded` : ""}`}
          {item.error && <span className="text-destructive"> · {item.error}</span>}
        </div>
      </div>
      {closed ? (
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={`Remove ${item.name} from the list`}
          title="Remove from list"
          onClick={() => {
            remove(item.id);
          }}
        >
          <X />
        </Button>
      ) : (
        <Button
          variant="outline"
          size="sm"
          disabled={item.status === "opening"}
          title="Stop uploading saves and delete the local copy"
          onClick={() => void stop(item.id)}
        >
          Done editing
        </Button>
      )}
    </li>
  );
}

function TransferRow({ item }: { item: TransferItem }) {
  const cancel = useTransferStore((s) => s.cancel);
  const retry = useTransferStore((s) => s.retry);
  const remove = useTransferStore((s) => s.remove);
  const finished = isFinished(item.status);
  const pct =
    item.bytesTotal > 0
      ? Math.min(100, (item.bytesDone / item.bytesTotal) * 100)
      : item.status === "done"
        ? 100
        : 0;

  return (
    <li className="flex items-center gap-3 border-b px-3 py-1.5 text-xs last:border-b-0">
      <StatusIcon status={item.status} />
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="truncate font-medium" title={item.title}>
            {item.title}
          </span>
          <span className="text-muted-foreground truncate" title={`${item.from} → ${item.to}`}>
            → {item.to}
          </span>
        </div>
        {!finished && (
          <div
            className="bg-muted mt-1 h-1.5 overflow-hidden rounded-full"
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(pct)}
            aria-label={`${item.title} progress`}
          >
            <div
              className="bg-brand h-full transition-[width]"
              style={{ width: `${String(pct)}%` }}
            />
          </div>
        )}
        <div className="text-muted-foreground mt-0.5 truncate">
          <Detail item={item} />
        </div>
      </div>
      <div className="flex shrink-0 gap-1">
        {!finished && (
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={`Cancel ${item.title}`}
            title="Cancel"
            onClick={() => void cancel(item.id)}
          >
            <Ban />
          </Button>
        )}
        {(item.status === "failed" || item.status === "cancelled") && (
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={`Retry ${item.title}`}
            title="Retry"
            onClick={() => void retry(item.id)}
          >
            <RotateCw />
          </Button>
        )}
        {finished && (
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={`Remove ${item.title} from the list`}
            title="Remove from list"
            onClick={() => {
              remove(item.id);
            }}
          >
            <X />
          </Button>
        )}
      </div>
    </li>
  );
}

function Detail({ item }: { item: TransferItem }) {
  switch (item.status) {
    case "queued":
      return <>Waiting…</>;
    case "running": {
      const eta = formatEta(item.bytesTotal - item.bytesDone, item.bytesPerSec);
      return (
        <>
          {formatBytes(item.bytesDone)} of {formatBytes(item.bytesTotal)}
          {item.filesTotal > 1 &&
            ` · file ${String(Math.min(item.filesDone + 1, item.filesTotal))} of ${String(item.filesTotal)}`}
          {item.bytesPerSec > 0 && ` · ${formatSpeed(item.bytesPerSec)}`}
          {eta && ` · ${eta} left`}
          {item.current && item.filesTotal > 1 && ` · ${item.current}`}
        </>
      );
    }
    case "done":
      return (
        <>
          Done · {item.filesDone} {item.filesDone === 1 ? "file" : "files"},{" "}
          {formatBytes(item.bytesDone)}
          {item.skipped > 0 && (
            <span className="text-amber-300" title={item.skippedNames.join("\n")}>
              {" "}
              · {item.skipped} skipped (already there, links, or names not allowed here)
            </span>
          )}
        </>
      );
    case "failed":
      return <span className="text-destructive">{item.message ?? "Failed."}</span>;
    case "cancelled":
      return <>Cancelled</>;
  }
}

function StatusIcon({ status }: { status: TransferItem["status"] }) {
  const cls = "size-4 shrink-0";
  switch (status) {
    case "queued":
      return <Clock className={cn(cls, "text-muted-foreground")} aria-label="Queued" />;
    case "running":
      return <ArrowRightLeft className={cn(cls, "text-brand")} aria-label="Copying" />;
    case "done":
      return <CheckCircle2 className={cn(cls, "text-emerald-400")} aria-label="Done" />;
    case "failed":
      return <XCircle className={cn(cls, "text-destructive")} aria-label="Failed" />;
    case "cancelled":
      return <Ban className={cn(cls, "text-muted-foreground")} aria-label="Cancelled" />;
  }
}
