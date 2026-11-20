import { useEffect, useState } from "react";
import { Loader2, SquareTerminal } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { IpcError, localShells } from "@/lib/ipc";
import { cn } from "@/lib/utils";
import { useLocalDialog } from "@/stores/localDialog";
import { useSessionStore } from "@/stores/session";
import type { LocalShell, LocalShellInfo } from "@/types/library";

const HINT: Record<LocalShell, string> = {
  powershell: "Windows PowerShell 5.1",
  pwsh: "PowerShell 7 (pwsh)",
  cmd: "cmd.exe",
  wsl: "Linux on Windows",
  default: "Your login shell",
};

/** Choose a shell for a new local terminal tab. */
export function LocalTerminalDialog() {
  const open = useLocalDialog((s) => s.open);
  const close = useLocalDialog((s) => s.close);
  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!o) close();
      }}
    >
      {open && <Picker onDone={close} />}
    </Dialog>
  );
}

function Picker({ onDone }: { onDone: () => void }) {
  const openLocal = useSessionStore((s) => s.openLocal);
  const [shells, setShells] = useState<LocalShellInfo[] | null>(null);
  const [shell, setShell] = useState<LocalShell | null>(null);
  const [distro, setDistro] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    localShells()
      .then((list) => {
        if (!live) return;
        setShells(list);
        setShell(list.find((s) => s.available)?.shell ?? null);
      })
      .catch(() => {
        if (live) setShells([]);
      });
    return () => {
      live = false;
    };
  }, []);

  const start = async () => {
    const info = shells?.find((s) => s.shell === shell);
    if (!info) return;
    setBusy(true);
    setError(null);
    try {
      const wslDistro = info.shell === "wsl" ? distro.trim() : "";
      await openLocal(
        { shell: info.shell, wslDistro },
        wslDistro ? `${info.label} · ${wslDistro}` : info.label,
      );
      onDone();
    } catch (err) {
      setError(err instanceof IpcError ? err.message : "Could not start the shell.");
      setBusy(false);
    }
  };

  return (
    <DialogContent className="sm:max-w-md">
      <DialogHeader>
        <DialogTitle className="flex items-center gap-2">
          <SquareTerminal className="size-4" /> New local terminal
        </DialogTitle>
        <DialogDescription>
          A shell on this computer, in a tab like any other. Ctrl+Shift+T opens the first one
          directly.
        </DialogDescription>
      </DialogHeader>
      {shells === null ? (
        <p className="text-muted-foreground flex items-center gap-2 text-sm">
          <Loader2 className="size-4 animate-spin" /> Looking for shells…
        </p>
      ) : (
        <div role="radiogroup" aria-label="Shell" className="grid gap-1.5">
          {shells.map((s) => (
            <button
              key={s.shell}
              type="button"
              role="radio"
              aria-checked={shell === s.shell}
              disabled={!s.available}
              onClick={() => {
                setShell(s.shell);
              }}
              onDoubleClick={() => {
                if (s.available) void start();
              }}
              className={cn(
                "flex items-center justify-between rounded-md border px-3 py-2 text-left text-sm outline-none disabled:opacity-50",
                "focus-visible:ring-ring hover:bg-accent focus-visible:ring-2",
                shell === s.shell && "border-brand bg-brand/10",
              )}
            >
              <span className="font-medium">{s.label}</span>
              <span className="text-muted-foreground text-xs">
                {s.available ? HINT[s.shell] : "not installed"}
              </span>
            </button>
          ))}
        </div>
      )}
      {shell === "wsl" && (
        <div className="grid gap-1.5">
          <Label htmlFor="lt-distro">Distribution</Label>
          <Input
            id="lt-distro"
            placeholder="Default (e.g. Ubuntu-24.04)"
            value={distro}
            maxLength={64}
            spellCheck={false}
            onChange={(e) => {
              setDistro(e.target.value);
            }}
          />
        </div>
      )}
      {error && (
        <p role="alert" className="text-destructive text-sm">
          {error}
        </p>
      )}
      <DialogFooter>
        <Button variant="outline" onClick={onDone}>
          Cancel
        </Button>
        <Button disabled={busy || shell === null} onClick={() => void start()}>
          {busy ? <Loader2 className="animate-spin" /> : <SquareTerminal />} Open
        </Button>
      </DialogFooter>
    </DialogContent>
  );
}
