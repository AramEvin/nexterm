import { useMemo, useState, type SyntheticEvent } from "react";
import { Check, Loader2, Save, Star } from "lucide-react";

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
import { buildIndex, folderPath, parseTags, SESSION_COLORS } from "@/lib/library";
import { cn } from "@/lib/utils";
import { useDialogStore, type EditorTarget } from "@/stores/dialogs";
import { toInput, useLibraryStore } from "@/stores/library";
import { useVaultStore } from "@/stores/vault";
import type { AuthMethod, Protocol, SessionInput } from "@/types/library";

/** Protocols shown in the picker; only SSH is available in this version. */
const PROTOCOLS: { value: string; label: string; available: boolean }[] = [
  { value: "ssh", label: "SSH", available: true },
  { value: "telnet", label: "Telnet (Phase 8)", available: false },
  { value: "serial", label: "Serial / COM (Phase 8)", available: false },
  { value: "local", label: "Local terminal (Phase 8)", available: false },
  { value: "rdp", label: "RDP (Phase 9)", available: false },
  { value: "vnc", label: "VNC (Phase 9)", available: false },
];

/** Which credential kind each login method uses (agent uses none). */
const CREDENTIAL_KIND: Record<AuthMethod, "password" | "key" | null> = {
  password: "password",
  publickey: "key",
  agent: null,
  "keyboard-interactive": "password",
};

const AUTH_HINT: Record<AuthMethod, string> = {
  password: "Without a saved credential you're asked for the password when connecting.",
  publickey: "Uses a private key stored in the vault.",
  agent: "Uses keys loaded in the Windows OpenSSH agent or Pageant; nothing is stored here.",
  "keyboard-interactive":
    "For servers with one-time codes. A saved password answers the password question automatically.",
};

const selectClass =
  "border-input bg-background dark:bg-input/30 h-9 w-full rounded-md border px-3 text-sm outline-none focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]";

function blankInput(folderId: string | null): SessionInput {
  return {
    folderId,
    name: "",
    protocol: "ssh",
    host: "",
    port: 22,
    username: "",
    authMethod: "password",
    credentialId: null,
    color: null,
    notes: "",
    favorite: false,
    tags: [],
  };
}

/** Create or edit a saved session. No password field: that comes with the vault. */
export function SessionEditorDialog() {
  const target = useDialogStore((s) => s.editor);
  const close = useDialogStore((s) => s.closeEditor);
  const [saving, setSaving] = useState(false);

  return (
    <Dialog
      open={target !== null}
      onOpenChange={(next) => {
        if (!next && !saving) close();
      }}
    >
      <DialogContent className="max-h-[92vh] overflow-y-auto sm:max-w-lg">
        {target && (
          <EditorForm
            key={target.mode === "edit" ? target.session.id : `new:${target.folderId ?? ""}`}
            target={target}
            saving={saving}
            setSaving={setSaving}
            onDone={close}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

interface EditorFormProps {
  target: EditorTarget;
  saving: boolean;
  setSaving: (saving: boolean) => void;
  onDone: () => void;
}

function EditorForm({ target, saving, setSaving, onDone }: EditorFormProps) {
  const folders = useLibraryStore((s) => s.folders);
  const sessions = useLibraryStore((s) => s.sessions);
  const createSession = useLibraryStore((s) => s.createSession);
  const updateSession = useLibraryStore((s) => s.updateSession);
  const credentials = useVaultStore((s) => s.credentials);
  const vaultStatus = useVaultStore((s) => s.status);
  const openCredentials = useDialogStore((s) => s.openCredentials);

  const initial = target.mode === "edit" ? toInput(target.session) : blankInput(target.folderId);
  const [form, setForm] = useState<SessionInput>(initial);
  const [port, setPort] = useState(String(initial.port));
  const [tags, setTags] = useState(initial.tags.join(", "));
  const [error, setError] = useState<string | null>(null);

  // Folder picker options, labelled by path.
  const folderOptions = useMemo(() => {
    const index = buildIndex(folders, sessions);
    return folders
      .map((f) => ({ id: f.id, path: folderPath(index, f.id) }))
      .sort((a, b) => a.path.localeCompare(b.path));
  }, [folders, sessions]);

  const update = (patch: Partial<SessionInput>) => {
    setForm((f) => ({ ...f, ...patch }));
  };

  async function onSubmit(e: SyntheticEvent<HTMLFormElement>) {
    e.preventDefault();
    const portNum = Number(port);
    if (!form.host.trim()) {
      setError("Host is required.");
      return;
    }
    if (!Number.isInteger(portNum) || portNum < 1 || portNum > 65535) {
      setError("Port must be between 1 and 65535.");
      return;
    }
    const input: SessionInput = {
      ...form,
      name: form.name.trim() || form.host.trim(),
      host: form.host.trim(),
      username: form.username.trim(),
      port: portNum,
      tags: parseTags(tags),
    };
    setSaving(true);
    setError(null);
    try {
      if (target.mode === "edit") await updateSession(target.session.id, input);
      else await createSession(input);
      onDone();
    } catch (err) {
      setError(err instanceof IpcError ? err.message : "Could not save the session.");
    } finally {
      setSaving(false);
    }
  }

  const editing = target.mode === "edit";

  return (
    <form onSubmit={(e) => void onSubmit(e)} className="grid gap-4" autoComplete="off">
      <DialogHeader>
        <DialogTitle>{editing ? "Edit session" : "New session"}</DialogTitle>
        <DialogDescription>
          Host, port and login options. Passwords and keys are stored encrypted in the vault, never
          in the session list.
        </DialogDescription>
      </DialogHeader>

      <div className="grid grid-cols-[1fr_10rem] gap-3">
        <div className="grid gap-1.5">
          <Label htmlFor="se-name">Name</Label>
          <Input
            id="se-name"
            autoFocus
            placeholder="Defaults to the host"
            value={form.name}
            maxLength={128}
            onChange={(e) => {
              update({ name: e.target.value });
            }}
          />
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="se-protocol">Protocol</Label>
          <select
            id="se-protocol"
            className={selectClass}
            value={form.protocol}
            onChange={(e) => {
              update({ protocol: e.target.value as Protocol });
            }}
          >
            {PROTOCOLS.map((p) => (
              <option key={p.value} value={p.value} disabled={!p.available}>
                {p.label}
              </option>
            ))}
          </select>
        </div>
      </div>

      <div className="grid grid-cols-[1fr_6rem] gap-3">
        <div className="grid gap-1.5">
          <Label htmlFor="se-host">Host</Label>
          <Input
            id="se-host"
            placeholder="server.example.com"
            value={form.host}
            spellCheck={false}
            onChange={(e) => {
              update({ host: e.target.value });
            }}
          />
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="se-port">Port</Label>
          <Input
            id="se-port"
            inputMode="numeric"
            value={port}
            onChange={(e) => {
              setPort(e.target.value);
            }}
          />
        </div>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <div className="grid gap-1.5">
          <Label htmlFor="se-user">Username</Label>
          <Input
            id="se-user"
            placeholder="Ask when connecting"
            value={form.username}
            spellCheck={false}
            onChange={(e) => {
              update({ username: e.target.value });
            }}
          />
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="se-folder">Folder</Label>
          <select
            id="se-folder"
            className={selectClass}
            value={form.folderId ?? ""}
            onChange={(e) => {
              update({ folderId: e.target.value || null });
            }}
          >
            <option value="">(top level)</option>
            {folderOptions.map((f) => (
              <option key={f.id} value={f.id}>
                {f.path}
              </option>
            ))}
          </select>
        </div>
      </div>

      <fieldset className="grid gap-2 rounded-md border p-3">
        <legend className="px-1 text-sm font-medium">Login</legend>
        <div className="grid grid-cols-2 gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="se-auth">Method</Label>
            <select
              id="se-auth"
              className={selectClass}
              value={form.authMethod}
              onChange={(e) => {
                const authMethod = e.target.value as AuthMethod;
                // A credential of the wrong kind doesn't apply to the new method.
                const keep = credentials.find((c) => c.id === form.credentialId);
                const fits = keep !== undefined && keep.kind === CREDENTIAL_KIND[authMethod];
                update({ authMethod, credentialId: fits ? form.credentialId : null });
              }}
            >
              <option value="password">Password</option>
              <option value="publickey">Private key</option>
              <option value="agent">SSH agent (OpenSSH / Pageant)</option>
              <option value="keyboard-interactive">Keyboard-interactive (2FA)</option>
            </select>
          </div>
          {form.authMethod !== "agent" && (
            <div className="grid gap-1.5">
              <Label htmlFor="se-cred">Saved credential</Label>
              <select
                id="se-cred"
                className={selectClass}
                value={form.credentialId ?? ""}
                onChange={(e) => {
                  update({ credentialId: e.target.value || null });
                }}
              >
                <option value="">
                  {form.authMethod === "publickey"
                    ? "(folder default)"
                    : "(folder default, or ask)"}
                </option>
                {credentials
                  .filter((c) => c.kind === CREDENTIAL_KIND[form.authMethod])
                  .map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.name}
                      {c.username ? ` (${c.username})` : ""}
                    </option>
                  ))}
              </select>
            </div>
          )}
        </div>
        <p className="text-muted-foreground text-xs">
          {AUTH_HINT[form.authMethod]}{" "}
          <button type="button" className="text-brand hover:underline" onClick={openCredentials}>
            {vaultStatus?.initialized === false
              ? "Set up the vault to save passwords and keys."
              : "Manage credentials…"}
          </button>
        </p>
      </fieldset>

      <div className="grid gap-1.5">
        <Label htmlFor="se-tags">Tags</Label>
        <Input
          id="se-tags"
          placeholder="prod, db, eu-west"
          value={tags}
          onChange={(e) => {
            setTags(e.target.value);
          }}
        />
      </div>

      <div className="grid gap-1.5">
        <span className="text-sm font-medium">Colour</span>
        <div className="flex items-center gap-2" role="radiogroup" aria-label="Colour">
          <button
            type="button"
            role="radio"
            aria-checked={form.color === null}
            aria-label="No colour"
            onClick={() => {
              update({ color: null });
            }}
            className={cn(
              "text-muted-foreground flex size-6 items-center justify-center rounded-full border text-xs",
              form.color === null && "ring-ring ring-2 ring-offset-1",
            )}
          >
            ∅
          </button>
          {SESSION_COLORS.map((c) => (
            <button
              key={c}
              type="button"
              role="radio"
              aria-checked={form.color === c}
              aria-label={c}
              onClick={() => {
                update({ color: c });
              }}
              className={cn(
                "flex size-6 items-center justify-center rounded-full",
                form.color === c && "ring-ring ring-2 ring-offset-1",
              )}
              style={{ backgroundColor: c }}
            >
              {form.color === c && <Check className="size-3.5 text-white" />}
            </button>
          ))}
        </div>
      </div>

      <div className="grid gap-1.5">
        <Label htmlFor="se-notes">Notes</Label>
        <textarea
          id="se-notes"
          rows={3}
          maxLength={4000}
          value={form.notes}
          onChange={(e) => {
            update({ notes: e.target.value });
          }}
          className="border-input dark:bg-input/30 focus-visible:border-ring focus-visible:ring-ring/50 rounded-md border bg-transparent px-3 py-2 text-sm outline-none focus-visible:ring-[3px]"
        />
      </div>

      <label className="flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          className="accent-brand size-4"
          checked={form.favorite}
          onChange={(e) => {
            update({ favorite: e.target.checked });
          }}
        />
        <Star className="size-3.5" /> Favourite
      </label>

      {error && (
        <p role="alert" className="text-destructive text-sm">
          {error}
        </p>
      )}

      <DialogFooter>
        <Button type="button" variant="outline" onClick={onDone} disabled={saving}>
          Cancel
        </Button>
        <Button type="submit" disabled={saving}>
          {saving ? <Loader2 className="animate-spin" /> : <Save />}
          {editing ? "Save" : "Create"}
        </Button>
      </DialogFooter>
    </form>
  );
}
