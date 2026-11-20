import { useMemo, useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  FileInput,
  FileKey2,
  Loader2,
  MonitorDown,
  Route,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { IpcError, importApply, importPreview } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useImportDialog } from "@/stores/importDialog";
import { useLibraryStore } from "@/stores/library";
import type { ImportFrom, ImportPreview, ImportResult, ImportedSession } from "@/types/library";

const SOURCE_LABEL = { mobaxterm: "MobaXterm", putty: "PuTTY" } as const;

/** Import saved sessions from MobaXterm or PuTTY: pick, review, import. */
export function ImportDialog() {
  const open = useImportDialog((s) => s.open);
  const close = useImportDialog((s) => s.close);
  // A fresh state every time the dialog opens.
  const [generation, setGeneration] = useState(0);
  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!o) {
          close();
          setGeneration((g) => g + 1);
        }
      }}
    >
      {open && <ImportFlow key={generation} onDone={close} />}
    </Dialog>
  );
}

type Step =
  | { kind: "choose"; busy: ImportFrom | null; error: string | null }
  | { kind: "review"; preview: ImportPreview }
  | { kind: "done"; result: ImportResult; source: string };

function ImportFlow({ onDone }: { onDone: () => void }) {
  const [step, setStep] = useState<Step>({ kind: "choose", busy: null, error: null });

  const pick = async (from: ImportFrom) => {
    setStep({ kind: "choose", busy: from, error: null });
    try {
      const preview = await importPreview(from);
      if (!preview) {
        setStep({ kind: "choose", busy: null, error: null });
      } else if (preview.sessions.length === 0 && preview.skipped.length === 0) {
        setStep({
          kind: "choose",
          busy: null,
          error:
            from === "putty-registry"
              ? "PuTTY has no saved sessions on this PC."
              : "No sessions found in that file.",
        });
      } else {
        setStep({ kind: "review", preview });
      }
    } catch (err) {
      setStep({
        kind: "choose",
        busy: null,
        error: err instanceof IpcError ? err.message : "Could not read the sessions.",
      });
    }
  };

  return (
    <DialogContent className="sm:max-w-3xl">
      <DialogHeader>
        <DialogTitle className="flex items-center gap-2">
          <FileInput className="size-4" /> Import sessions
        </DialogTitle>
        <DialogDescription>
          Bring saved sessions over from MobaXterm or PuTTY. Folders and jump hosts come along;
          passwords don&apos;t (link credentials after importing).
        </DialogDescription>
      </DialogHeader>
      {step.kind === "choose" && (
        <Choose busy={step.busy} error={step.error} onPick={(f) => void pick(f)} />
      )}
      {step.kind === "review" && (
        <Review
          preview={step.preview}
          onBack={() => {
            setStep({ kind: "choose", busy: null, error: null });
          }}
          onImported={(result) => {
            setStep({ kind: "done", result, source: SOURCE_LABEL[step.preview.source] });
          }}
        />
      )}
      {step.kind === "done" && <Done result={step.result} source={step.source} onClose={onDone} />}
    </DialogContent>
  );
}

function Choose({
  busy,
  error,
  onPick,
}: {
  busy: ImportFrom | null;
  error: string | null;
  onPick: (from: ImportFrom) => void;
}) {
  const options: { from: ImportFrom; title: string; text: string; icon: React.ReactNode }[] = [
    {
      from: "mobaxterm-file",
      title: "MobaXterm",
      text: "A .mxtsessions export (right-click User sessions → Export) or MobaXterm.ini.",
      icon: <FileInput className="size-5" />,
    },
    {
      from: "putty-registry",
      title: "PuTTY on this PC",
      text: "Reads PuTTY's saved sessions from the Windows registry. Nothing is changed there.",
      icon: <MonitorDown className="size-5" />,
    },
    {
      from: "putty-file",
      title: "PuTTY .reg file",
      text: "A registry export of HKCU\\Software\\SimonTatham\\PuTTY\\Sessions from another PC.",
      icon: <FileKey2 className="size-5" />,
    },
  ];
  return (
    <div className="space-y-3">
      <div className="grid gap-2 sm:grid-cols-3">
        {options.map((o) => (
          <button
            key={o.from}
            type="button"
            disabled={busy !== null}
            onClick={() => {
              onPick(o.from);
            }}
            className="hover:border-brand/60 hover:bg-accent focus-visible:ring-ring flex flex-col items-start gap-2 rounded-lg border p-3 text-left outline-none focus-visible:ring-2 disabled:opacity-60"
          >
            <span className="flex items-center gap-2 font-medium">
              {busy === o.from ? <Loader2 className="size-5 animate-spin" /> : o.icon}
              {o.title}
            </span>
            <span className="text-muted-foreground text-xs [overflow-wrap:anywhere]">{o.text}</span>
          </button>
        ))}
      </div>
      {error && (
        <p role="alert" className="text-destructive flex items-center gap-2 text-sm">
          <AlertTriangle className="size-4" /> {error}
        </p>
      )}
    </div>
  );
}

function sessionKey(s: ImportedSession, i: number): string {
  return `${String(i)}:${s.name}`;
}

function Review({
  preview,
  onBack,
  onImported,
}: {
  preview: ImportPreview;
  onBack: () => void;
  onImported: (r: ImportResult) => void;
}) {
  const library = useLibraryStore((s) => s.sessions);
  const reload = useLibraryStore((s) => s.load);
  const [selected, setSelected] = useState<Set<string>>(
    () => new Set(preview.sessions.map(sessionKey)),
  );
  const [skipExisting, setSkipExisting] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const existing = useMemo(() => {
    const keys = new Set(
      library.map(
        (s) =>
          `${s.protocol}|${s.host.toLowerCase()}|${String(s.port)}|${s.username.toLowerCase()}`,
      ),
    );
    return (s: ImportedSession) =>
      keys.has(
        `${s.protocol}|${s.host.toLowerCase()}|${String(s.port)}|${s.username.toLowerCase()}`,
      );
  }, [library]);

  const chosen = preview.sessions.filter((s, i) => selected.has(sessionKey(s, i)));
  const allChecked = chosen.length === preview.sessions.length;
  const source = SOURCE_LABEL[preview.source];

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const result = await importApply(preview.source, chosen, skipExisting);
      await reload();
      onImported(result);
    } catch (err) {
      setError(err instanceof IpcError ? err.message : "The import failed.");
      setBusy(false);
    }
  };

  return (
    <>
      <div className="flex items-center gap-3 text-sm">
        <span>
          <strong>{preview.sessions.length}</strong> session
          {preview.sessions.length === 1 ? "" : "s"} found in {source}
          {preview.skipped.length > 0 && `, ${String(preview.skipped.length)} can't be imported`}.
        </span>
        <label className="ml-auto flex items-center gap-2">
          <input
            type="checkbox"
            className="accent-brand size-4"
            checked={allChecked}
            onChange={() => {
              setSelected(allChecked ? new Set() : new Set(preview.sessions.map(sessionKey)));
            }}
          />
          Select all
        </label>
      </div>

      <div className="max-h-80 overflow-auto rounded-md border">
        <table className="w-full text-sm">
          <thead className="bg-muted/60 text-muted-foreground sticky top-0 text-left text-xs">
            <tr>
              <th className="w-8 px-2 py-1.5" />
              <th className="px-2 py-1.5 font-medium">Name</th>
              <th className="px-2 py-1.5 font-medium">Folder</th>
              <th className="px-2 py-1.5 font-medium">Server</th>
              <th className="px-2 py-1.5 font-medium">Notes</th>
            </tr>
          </thead>
          <tbody>
            {preview.sessions.map((s, i) => {
              const k = sessionKey(s, i);
              const dupe = existing(s);
              return (
                <tr key={k} className={cn("border-t", dupe && skipExisting && "opacity-50")}>
                  <td className="px-2 py-1">
                    <input
                      type="checkbox"
                      aria-label={`Import ${s.name}`}
                      className="accent-brand size-4"
                      checked={selected.has(k)}
                      onChange={() => {
                        setSelected((prev) => {
                          const next = new Set(prev);
                          if (next.has(k)) next.delete(k);
                          else next.add(k);
                          return next;
                        });
                      }}
                    />
                  </td>
                  <td className="max-w-48 truncate px-2 py-1" title={s.name}>
                    {s.name}
                    {s.protocol !== "ssh" && (
                      <span className="text-muted-foreground ml-1.5 rounded border px-1 text-[10px] uppercase">
                        {s.protocol}
                      </span>
                    )}
                  </td>
                  <td className="text-muted-foreground max-w-40 truncate px-2 py-1">
                    {s.folder.join(" / ")}
                  </td>
                  <td className="max-w-56 truncate px-2 py-1 font-mono text-xs">
                    {s.username ? `${s.username}@` : ""}
                    {s.host}
                    {s.port === 22 || (s.protocol === "ftp" && s.port === 21)
                      ? ""
                      : `:${String(s.port)}`}
                  </td>
                  <td className="text-muted-foreground px-2 py-1 text-xs">
                    <span className="flex flex-wrap items-center gap-x-2">
                      {s.jump.length > 0 && (
                        <span className="flex items-center gap-1" title="Jump hosts">
                          <Route className="size-3" />
                          via {s.jump.map((h) => h.host).join(" → ")}
                        </span>
                      )}
                      {s.keyPath && <span title={s.keyPath}>key file</span>}
                      {dupe && <span>already saved</span>}
                    </span>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      {preview.skipped.length > 0 && (
        <details className="text-sm">
          <summary className="text-muted-foreground cursor-pointer">
            {preview.skipped.length} not imported (why)
          </summary>
          <ul className="text-muted-foreground mt-1 max-h-32 space-y-0.5 overflow-auto pl-4 text-xs">
            {preview.skipped.map((s, i) => (
              <li key={`${String(i)}:${s.name}`}>
                <span className="text-foreground">{s.name}</span>: {s.reason}
              </li>
            ))}
          </ul>
        </details>
      )}

      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          className="accent-brand size-4"
          checked={skipExisting}
          onChange={(e) => {
            setSkipExisting(e.target.checked);
          }}
        />
        Skip sessions already saved in NexTerm (same server, port and user)
      </label>
      {error && (
        <p role="alert" className="text-destructive text-sm">
          {error}
        </p>
      )}

      <DialogFooter>
        <Button variant="outline" onClick={onBack} disabled={busy} className="mr-auto">
          Back
        </Button>
        <Button disabled={busy || chosen.length === 0} onClick={() => void run()}>
          {busy ? <Loader2 className="animate-spin" /> : <FileInput />}
          Import {chosen.length} session{chosen.length === 1 ? "" : "s"}
        </Button>
      </DialogFooter>
    </>
  );
}

function Done({
  result,
  source,
  onClose,
}: {
  result: ImportResult;
  source: string;
  onClose: () => void;
}) {
  return (
    <>
      <div className="flex gap-3 text-sm">
        <CheckCircle2 className="size-5 shrink-0 text-emerald-400" />
        <div className="space-y-1">
          <p>
            <strong>{result.created}</strong> session{result.created === 1 ? "" : "s"} added to the
            folder <strong>Imported from {source}</strong>
            {result.jumpHosts > 0 &&
              `, plus ${String(result.jumpHosts)} jump host${result.jumpHosts === 1 ? "" : "s"}`}
            .
          </p>
          {result.existing > 0 && (
            <p className="text-muted-foreground">{result.existing} already saved, so skipped.</p>
          )}
          {result.failed > 0 && (
            <p className="text-muted-foreground">{result.failed} couldn&apos;t be saved.</p>
          )}
          <p className="text-muted-foreground">
            Next: link a saved password or key to each session (or to the folder, as its default) so
            you don&apos;t have to type it. Jump hosts need one of their own.
          </p>
        </div>
      </div>
      <DialogFooter>
        <Button onClick={onClose}>Done</Button>
      </DialogFooter>
    </>
  );
}
