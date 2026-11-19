import { useState, type SyntheticEvent } from "react";
import { Loader2, Server, Zap } from "lucide-react";

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
import { handleConnectError } from "@/lib/connect";
import { IpcError } from "@/lib/ipc";
import { useDialogStore, type ConnectTarget } from "@/stores/dialogs";
import { useLibraryStore } from "@/stores/library";
import { useSessionStore } from "@/stores/session";

/** Client-side checks for fast feedback. Rust re-validates everything. */
function validate(host: string, port: string, username: string): string | null {
  if (!host.trim()) return "Host is required.";
  if (!/^\[?[A-Za-z0-9.\-_:]+\]?$/.test(host.trim())) return "Host contains invalid characters.";
  const p = Number(port);
  if (!Number.isInteger(p) || p < 1 || p > 65535) return "Port must be between 1 and 65535.";
  if (!username.trim()) return "Username is required.";
  return null;
}

/**
 * Quick connect (type a host) or connect to a saved session (only the password
 * is asked). The password is sent once and cleared after every attempt; it is
 * never kept by the UI (store it as a vault credential to reuse it).
 */
export function ConnectDialog() {
  const target = useDialogStore((s) => s.connect);
  const close = useDialogStore((s) => s.closeConnect);
  const connecting = useSessionStore((s) => s.connecting);

  return (
    <Dialog
      open={target !== null}
      onOpenChange={(next) => {
        if (!next && !connecting) close();
      }}
    >
      <DialogContent
        className="sm:max-w-md"
        onCloseAutoFocus={(e) => {
          // Radix would refocus the opener; send keys to the terminal instead.
          const { tabs, activeTabId } = useSessionStore.getState();
          const tab = tabs.find((t) => t.id === activeTabId);
          if (tab) {
            e.preventDefault();
            tab.controller.focus();
          }
        }}
      >
        {target && (
          <ConnectForm
            key={target.mode === "saved" ? target.session.id : "quick"}
            target={target}
            onDone={close}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

/** Last quick-connect values, remembered for the next opening (never the password). */
let lastQuick = { host: "", port: "22", username: "" };

function ConnectForm({ target, onDone }: { target: ConnectTarget; onDone: () => void }) {
  const connect = useSessionStore((s) => s.connect);
  const connectSaved = useSessionStore((s) => s.connectSaved);
  const connecting = useSessionStore((s) => s.connecting);
  const createSession = useLibraryStore((s) => s.createSession);
  const reloadLibrary = useLibraryStore((s) => s.load);

  const saved = target.mode === "saved" ? target.session : null;
  const initial = saved
    ? { host: saved.host, port: String(saved.port), username: saved.username }
    : lastQuick;
  const [host, setHost] = useState(initial.host);
  const [port, setPort] = useState(initial.port);
  const [username, setUsername] = useState(initial.username);
  const [password, setPassword] = useState("");
  const [saveSession, setSaveSession] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function onSubmit(e: SyntheticEvent<HTMLFormElement>) {
    e.preventDefault();
    const problem = validate(host, port, username);
    if (problem) {
      setError(problem);
      return;
    }
    setError(null);
    const form = { host: host.trim(), port: Number(port), username: username.trim(), password };
    try {
      if (saved) {
        // Rust still resolves host and any stored credential; we only add
        // what it asked for.
        await connectSaved(saved, { username: form.username, password: password || undefined });
        void reloadLibrary();
      } else {
        lastQuick = { host: form.host, port: String(form.port), username: form.username };
        await connect(form);
        if (saveSession) {
          // Saves host/port/user only; never the password.
          await createSession({
            folderId: null,
            name: `${form.username}@${form.host}`,
            protocol: "ssh",
            host: form.host,
            port: form.port,
            username: form.username,
            authMethod: "password",
            credentialId: null,
            color: null,
            notes: "",
            favorite: false,
            resourceMonitor: true,
            tags: [],
          }).catch(() => undefined);
        }
      }
      onDone();
    } catch (err) {
      if (!(err instanceof IpcError)) {
        setError("Connection failed.");
      } else if (
        err.code === "VAULT_LOCKED" ||
        err.code === "HOST_KEY_CHANGED" ||
        err.code === "HOST_KEY_REJECTED" ||
        err.code === "CANCELLED"
      ) {
        // Another dialog handles these (or the user chose to stop).
        onDone();
        handleConnectError(err.code, err.message, {
          host: form.host,
          port: form.port,
          retry: () => {
            useDialogStore.getState().openConnect(target);
          },
        });
      } else {
        setError(err.message);
      }
    } finally {
      setPassword("");
    }
  }

  return (
    <form onSubmit={(e) => void onSubmit(e)} className="grid gap-4" autoComplete="off">
      <DialogHeader>
        <DialogTitle>{saved ? `Connect to ${saved.name}` : "Quick connect"}</DialogTitle>
        <DialogDescription>
          {saved ? (
            <span className="flex items-center gap-1.5">
              <Server className="size-3.5" /> {saved.host}:{saved.port}
            </span>
          ) : (
            "Open an SSH session with a password."
          )}
        </DialogDescription>
      </DialogHeader>

      {!saved && (
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
      )}
      <div className="grid gap-1.5">
        <Label htmlFor="qc-user">Username</Label>
        <Input
          id="qc-user"
          placeholder="root"
          autoFocus={saved !== null && !saved.username}
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
          autoFocus={saved !== null && Boolean(saved.username)}
          value={password}
          onChange={(e) => {
            setPassword(e.target.value);
          }}
          disabled={connecting}
        />
      </div>

      {!saved && (
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            className="accent-brand size-4"
            checked={saveSession}
            onChange={(e) => {
              setSaveSession(e.target.checked);
            }}
            disabled={connecting}
          />
          Save to sessions (without the password)
        </label>
      )}

      {error && (
        <p role="alert" className="text-destructive text-sm">
          {error}
        </p>
      )}

      <p className="text-muted-foreground text-xs">
        {saved
          ? "To skip this next time, link a saved credential in the session's Login settings."
          : "The password is used once and not saved. Save it as a vault credential to reuse it."}
      </p>

      <DialogFooter>
        <Button type="submit" disabled={connecting}>
          {connecting ? <Loader2 className="animate-spin" /> : <Zap />}
          {connecting ? "Connecting…" : "Connect"}
        </Button>
      </DialogFooter>
    </form>
  );
}
