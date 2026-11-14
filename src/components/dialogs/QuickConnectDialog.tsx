import { useState, type SyntheticEvent } from "react";
import { Loader2, Zap } from "lucide-react";

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
import { IpcError } from "@/lib/ipc";
import { useSessionStore } from "@/stores/session";

interface QuickConnectDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/** Client-side checks for fast feedback. Rust re-validates everything. */
function validate(host: string, port: string, username: string): string | null {
  if (!host.trim()) return "Host is required.";
  if (!/^\[?[A-Za-z0-9.\-_:]+\]?$/.test(host.trim())) return "Host contains invalid characters.";
  const p = Number(port);
  if (!Number.isInteger(p) || p < 1 || p > 65535) return "Port must be between 1 and 65535.";
  if (!username.trim()) return "Username is required.";
  return null;
}

export function QuickConnectDialog({ open, onOpenChange }: QuickConnectDialogProps) {
  const connect = useSessionStore((s) => s.connect);
  const connecting = useSessionStore((s) => s.connecting);

  // Host/port/user are kept for convenience; the password is cleared after every attempt.
  const [host, setHost] = useState("");
  const [port, setPort] = useState("22");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);

  async function onSubmit(e: SyntheticEvent<HTMLFormElement>) {
    e.preventDefault();
    const problem = validate(host, port, username);
    if (problem) {
      setError(problem);
      return;
    }
    setError(null);
    try {
      await connect({ host: host.trim(), port: Number(port), username, password });
      onOpenChange(false);
    } catch (err) {
      setError(err instanceof IpcError ? err.message : "Connection failed.");
    } finally {
      setPassword("");
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!connecting) onOpenChange(next);
      }}
    >
      <DialogContent
        className="sm:max-w-md"
        onCloseAutoFocus={(e) => {
          // Radix would refocus the "Quick connect" button; send keys to the terminal instead.
          const { tabs, activeTabId } = useSessionStore.getState();
          const session = tabs.find((t) => t.id === activeTabId);
          if (session) {
            e.preventDefault();
            session.controller.focus();
          }
        }}
      >
        <form onSubmit={(e) => void onSubmit(e)} className="grid gap-4" autoComplete="off">
          <DialogHeader>
            <DialogTitle>Quick connect</DialogTitle>
            <DialogDescription>Open an SSH session with a password.</DialogDescription>
          </DialogHeader>

          <div className="grid grid-cols-[1fr_6rem] gap-3">
            <div className="grid gap-1.5">
              <Label htmlFor="qc-host">Host</Label>
              <Input
                id="qc-host"
                autoFocus
                placeholder="server.example.com"
                value={host}
                onChange={(e) => {
                  setHost(e.target.value);
                }}
                disabled={connecting}
                spellCheck={false}
              />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="qc-port">Port</Label>
              <Input
                id="qc-port"
                inputMode="numeric"
                value={port}
                onChange={(e) => {
                  setPort(e.target.value);
                }}
                disabled={connecting}
              />
            </div>
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="qc-user">Username</Label>
            <Input
              id="qc-user"
              placeholder="root"
              value={username}
              onChange={(e) => {
                setUsername(e.target.value);
              }}
              disabled={connecting}
              spellCheck={false}
            />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="qc-pass">Password</Label>
            <Input
              id="qc-pass"
              type="password"
              autoComplete="new-password"
              value={password}
              onChange={(e) => {
                setPassword(e.target.value);
              }}
              disabled={connecting}
            />
          </div>

          {error && (
            <p role="alert" className="text-destructive text-sm">
              {error}
            </p>
          )}

          <p className="text-muted-foreground text-xs">
            Host keys are not verified yet (coming in Phase 4). Only connect over networks you
            trust.
          </p>

          <DialogFooter>
            <Button type="submit" disabled={connecting}>
              {connecting ? <Loader2 className="animate-spin" /> : <Zap />}
              {connecting ? "Connecting…" : "Connect"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
