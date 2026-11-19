import { useState } from "react";
import { Circle, Radio } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import { useMultiExecDialog } from "@/stores/multiExecDialog";
import { panesOf, tabTitle, useSessionStore, type SessionTab } from "@/stores/session";

/**
 * Turning multi-exec on is an explicit choice: pick the terminals, confirm.
 * While it's on, every key typed in one of them goes to all of them.
 */
export function MultiExecDialog() {
  const openFor = useMultiExecDialog((s) => s.openFor);
  const close = useMultiExecDialog((s) => s.close);
  return (
    <Dialog
      open={openFor !== null}
      onOpenChange={(open) => {
        if (!open) close();
      }}
    >
      {openFor !== null && <Picker key={openFor} tabId={openFor} onDone={close} />}
    </Dialog>
  );
}

function Picker({ tabId, onDone }: { tabId: string; onDone: () => void }) {
  const tabs = useSessionStore((s) => s.tabs);
  const layouts = useSessionStore((s) => s.layouts);
  const current = useSessionStore((s) => s.multiExec);
  const setMultiExec = useSessionStore((s) => s.setMultiExec);

  const roots = tabs.filter((t) => t.parentId === null);
  const groups = roots.map((root) => ({ root, panes: panesOf({ tabs, layouts }, root.id) }));
  const [selected, setSelected] = useState<Set<string>>(() => {
    if (current.length > 0) return new Set(current);
    // Default: the connected terminals of the tab it was opened from.
    const own = panesOf({ tabs, layouts }, tabId).filter((t) => t.state === "connected");
    return new Set(own.map((t) => t.id));
  });

  const toggle = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };
  const count = selected.size;

  return (
    <DialogContent
      className="sm:max-w-md"
      onOpenAutoFocus={(e) => {
        e.preventDefault();
        document.getElementById("multiexec-cancel")?.focus();
      }}
    >
      <DialogHeader>
        <DialogTitle className="flex items-center gap-2">
          <Radio className="size-4 text-orange-400" /> Multi-exec
        </DialogTitle>
        <DialogDescription>
          Everything you type or paste in one of these terminals is sent to all of them. Pick at
          least two. Their borders turn orange while it's on.
        </DialogDescription>
      </DialogHeader>

      <div className="max-h-80 space-y-3 overflow-y-auto pr-1">
        {groups.map(({ root, panes }) => (
          <fieldset key={root.id} className="space-y-1">
            <legend className="text-muted-foreground mb-1 text-xs font-medium">
              {tabTitle(root)}
              {panes.length > 1 ? ` (${String(panes.length)} panes)` : ""}
            </legend>
            {panes.map((p, i) => (
              <PaneOption
                key={p.id}
                pane={p}
                index={panes.length > 1 ? i + 1 : null}
                checked={selected.has(p.id)}
                onToggle={() => {
                  toggle(p.id);
                }}
              />
            ))}
          </fieldset>
        ))}
      </div>

      <DialogFooter>
        {current.length > 0 && (
          <Button
            variant="outline"
            className="mr-auto"
            onClick={() => {
              setMultiExec([]);
              onDone();
            }}
          >
            Turn off
          </Button>
        )}
        <Button id="multiexec-cancel" variant="outline" onClick={onDone}>
          Cancel
        </Button>
        <Button
          disabled={count < 2}
          className="bg-orange-500 text-white hover:bg-orange-500/90"
          onClick={() => {
            setMultiExec([...selected]);
            onDone();
          }}
        >
          <Radio /> {current.length > 0 ? "Update" : "Start"} ({count})
        </Button>
      </DialogFooter>
    </DialogContent>
  );
}

function PaneOption({
  pane,
  index,
  checked,
  onToggle,
}: {
  pane: SessionTab;
  index: number | null;
  checked: boolean;
  onToggle: () => void;
}) {
  const connected = pane.state === "connected";
  return (
    <label
      className={cn(
        "hover:bg-accent flex items-center gap-2 rounded px-2 py-1 text-sm",
        !connected && "opacity-60",
      )}
    >
      <input
        type="checkbox"
        className="size-4 accent-orange-500"
        checked={checked}
        disabled={!connected && !checked}
        onChange={onToggle}
      />
      <Circle
        className={cn(
          "size-2 fill-current",
          connected
            ? "text-emerald-400"
            : pane.state === "connecting"
              ? "text-amber-400"
              : "text-destructive",
        )}
      />
      <span className="truncate">
        {index !== null && <span className="text-muted-foreground">Pane {index}: </span>}
        {pane.username}@{pane.host}
      </span>
      {!connected && <span className="text-muted-foreground ml-auto text-xs">not connected</span>}
    </label>
  );
}
