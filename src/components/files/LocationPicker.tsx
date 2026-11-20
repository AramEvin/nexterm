import { useDeferredValue, useMemo, useState } from "react";
import { HardDrive, Search, Server, TerminalSquare } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useLibraryStore } from "@/stores/library";
import { tabTitle, useSessionStore } from "@/stores/session";
import type { PaneSource } from "@/stores/files";

/** Most saved sessions listed at once (the search narrows it down). */
const MAX_SHOWN = 200;

/** Where can a pane point: this PC, an open terminal's server, or a saved session. */
export function LocationPicker({
  onPick,
  onCancel,
}: {
  onPick: (source: PaneSource) => void;
  onCancel?: () => void;
}) {
  const sessions = useLibraryStore((s) => s.sessions);
  const tabs = useSessionStore((s) => s.tabs);
  const [query, setQuery] = useState("");
  const q = useDeferredValue(query.trim().toLowerCase());

  // Only SSH terminals can lend their login to SFTP.
  const terminals = tabs.filter((t) => t.connectionId !== null && t.protocol === "ssh");
  const matches = useMemo(() => {
    // SSH sessions open over SFTP; FTP/FTPS sessions directly.
    const ssh = sessions.filter(
      (s) => s.protocol === "ssh" || s.protocol === "ftp" || s.protocol === "ftps",
    );
    const hit = q
      ? ssh.filter((s) =>
          [s.name, s.host, s.username, ...s.tags].some((v) => v.toLowerCase().includes(q)),
        )
      : ssh;
    return [...hit].sort(
      (a, b) =>
        Number(b.favorite) - Number(a.favorite) ||
        (b.lastConnectedAt ?? 0) - (a.lastConnectedAt ?? 0) ||
        a.name.localeCompare(b.name),
    );
  }, [sessions, q]);

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-auto p-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-medium">Open a location</h3>
        {onCancel && (
          <Button variant="ghost" size="sm" onClick={onCancel}>
            Cancel
          </Button>
        )}
      </div>

      <Choice
        icon={<HardDrive className="text-brand size-4" />}
        title="Local disk"
        detail="Files on this PC"
        onClick={() => {
          onPick({ kind: "local" });
        }}
      />

      {terminals.length > 0 && (
        <Section title="Open terminals">
          {terminals.map((t) => (
            <Choice
              key={t.id}
              icon={<TerminalSquare className="size-4 text-emerald-400" />}
              title={tabTitle(t)}
              detail={`${t.username}@${t.host}:${String(t.port)} · same login, no password needed`}
              onClick={() => {
                if (t.connectionId)
                  onPick({
                    kind: "connection",
                    connectionId: t.connectionId,
                    name: tabTitle(t),
                    host: t.host,
                    port: t.port,
                  });
              }}
            />
          ))}
        </Section>
      )}

      <Section title="Saved sessions">
        <div className="relative">
          <Search className="text-muted-foreground absolute top-1/2 left-2 size-3.5 -translate-y-1/2" />
          <Input
            autoFocus
            value={query}
            placeholder="Search name, host, user, tag"
            aria-label="Search saved sessions"
            className="h-8 pl-7 text-sm"
            onChange={(e) => {
              setQuery(e.target.value);
            }}
          />
        </div>
        {matches.length === 0 && (
          <p className="text-muted-foreground px-1 text-sm">
            {sessions.length === 0 ? "No saved sessions yet." : "No matches."}
          </p>
        )}
        {matches.slice(0, MAX_SHOWN).map((s) => (
          <Choice
            key={s.id}
            icon={<Server className="text-muted-foreground size-4" />}
            accent={s.color}
            title={s.name}
            detail={`${s.protocol === "ssh" ? "SFTP" : s.protocol === "ftps" ? "FTPS" : "FTP (unencrypted)"} · ${s.username ? `${s.username}@` : ""}${s.host}:${String(s.port)}`}
            onClick={() => {
              onPick({ kind: "saved", sessionId: s.id, name: s.name, host: s.host, port: s.port });
            }}
          />
        ))}
        {matches.length > MAX_SHOWN && (
          <p className="text-muted-foreground px-1 text-xs">
            {matches.length - MAX_SHOWN} more — refine the search.
          </p>
        )}
      </Section>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="grid gap-1">
      <h4 className="text-muted-foreground px-1 text-xs font-medium tracking-wide uppercase">
        {title}
      </h4>
      {children}
    </div>
  );
}

function Choice({
  icon,
  title,
  detail,
  accent,
  onClick,
}: {
  icon: React.ReactNode;
  title: string;
  detail: string;
  accent?: string | null;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="hover:bg-accent focus-visible:ring-ring flex items-center gap-3 rounded-md border px-3 py-2 text-left outline-none focus-visible:ring-2"
      style={accent ? { borderLeftColor: accent, borderLeftWidth: 3 } : undefined}
    >
      {icon}
      <span className="min-w-0">
        <span className="block truncate text-sm">{title}</span>
        <span className="text-muted-foreground block truncate text-xs">{detail}</span>
      </span>
    </button>
  );
}
