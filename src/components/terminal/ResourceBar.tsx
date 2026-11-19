import { useEffect, useState } from "react";
import { Activity, Cpu, EyeOff, HardDrive, MemoryStick, Timer } from "lucide-react";

import { sshMonitorStart, sshMonitorStop } from "@/lib/ipc";
import {
  diskPercent,
  formatKib,
  formatUptime,
  level,
  loadLevel,
  percent,
  type Level,
} from "@/lib/resources";
import { cn } from "@/lib/utils";
import { useSessionStore, type SessionTab } from "@/stores/session";
import type { ResourceEvent, ResourceSample } from "@/types/ipc";

type View =
  | { kind: "waiting" }
  | { kind: "sample"; sample: ResourceSample }
  | { kind: "message"; text: string };

const LEVEL_BAR: Record<Level, string> = {
  ok: "bg-emerald-400",
  warn: "bg-amber-400",
  critical: "bg-red-500",
};

const LEVEL_TEXT: Record<Level, string> = {
  ok: "",
  warn: "text-amber-300",
  critical: "text-red-400 font-medium",
};

/**
 * Start/stop calls run concurrently on the Rust side, so a quick hide/show
 * could deliver "start" before "stop". Chain them per session instead.
 */
const chains = new Map<string, Promise<unknown>>();

function inOrder(sessionId: string, call: () => Promise<unknown>): Promise<unknown> {
  const next = (chains.get(sessionId) ?? Promise.resolve()).catch(() => undefined).then(call);
  chains.set(sessionId, next);
  void next
    .finally(() => {
      if (chains.get(sessionId) === next) chains.delete(sessionId);
    })
    .catch(() => undefined);
  return next;
}

/**
 * Live CPU / RAM / disk / load / uptime of the server under a terminal.
 * Readings come from a separate exec channel (nothing runs in the shell) and
 * only while the terminal is visible.
 */
export function ResourceBar({ tab, visible }: { tab: SessionTab; visible: boolean }) {
  const setResourceMonitor = useSessionStore((s) => s.setResourceMonitor);
  const [view, setView] = useState<View>({ kind: "waiting" });
  const sessionId = tab.state === "connected" ? tab.sessionId : null;

  useEffect(() => {
    if (!sessionId || !visible) return;
    let live = true;
    const onEvent = (ev: ResourceEvent) => {
      if (!live) return;
      if (ev.kind === "sample") setView({ kind: "sample", sample: ev });
      else setView({ kind: "message", text: ev.reason });
    };
    inOrder(sessionId, () => sshMonitorStart(sessionId, onEvent)).catch(() => {
      if (live) setView({ kind: "message", text: "Resource readings are not available." });
    });
    return () => {
      live = false;
      void inOrder(sessionId, () => sshMonitorStop(sessionId)).catch(() => undefined);
    };
  }, [sessionId, visible]);

  if (!sessionId) return null;

  return (
    <div
      role="status"
      aria-label={`Server resources of ${tab.host}`}
      className="bg-sidebar text-muted-foreground border-sidebar-border flex h-6 shrink-0 items-center gap-4 overflow-hidden border-t px-2 text-[11px] tabular-nums"
    >
      {view.kind === "waiting" && <span>Reading server resources…</span>}
      {view.kind === "message" && <span className="truncate">{view.text}</span>}
      {view.kind === "sample" && <Readings s={view.sample} />}
      <button
        type="button"
        aria-label="Hide resource bar"
        title="Hide for this terminal (turn off for good in the session settings)"
        className="hover:text-foreground ml-auto shrink-0 rounded p-0.5"
        onClick={() => {
          setResourceMonitor(tab.id, false);
        }}
      >
        <EyeOff className="size-3" />
      </button>
    </div>
  );
}

function Readings({ s }: { s: ResourceSample }) {
  const memPct = percent(s.memUsedKib, s.memTotalKib);
  const diskPct =
    s.diskUsedKib !== null && s.diskAvailKib !== null
      ? diskPercent(s.diskUsedKib, s.diskAvailKib)
      : null;
  return (
    <>
      <Meter
        icon={<Cpu className="size-3" />}
        label="CPU"
        pct={s.cpuPercent}
        text={s.cpuPercent === null ? "…" : `${s.cpuPercent.toFixed(0)}%`}
        title={`${String(s.cores)} cores`}
      />
      <Meter
        icon={<MemoryStick className="size-3" />}
        label="RAM"
        pct={memPct}
        text={`${formatKib(s.memUsedKib)} / ${formatKib(s.memTotalKib)}`}
        title={
          s.swapTotalKib > 0
            ? `Swap ${formatKib(s.swapUsedKib)} / ${formatKib(s.swapTotalKib)}`
            : "No swap"
        }
      />
      {diskPct !== null && s.diskUsedKib !== null && s.diskAvailKib !== null && (
        <Meter
          icon={<HardDrive className="size-3" />}
          label="Disk /"
          pct={diskPct}
          text={`${diskPct.toFixed(0)}% · ${formatKib(s.diskAvailKib)} free`}
          title={`${formatKib(s.diskUsedKib)} used of ${formatKib(s.diskTotalKib ?? 0)}`}
        />
      )}
      <span
        className={cn(
          "flex shrink-0 items-center gap-1",
          LEVEL_TEXT[loadLevel(s.load[0], s.cores)],
        )}
        title="Load average (1, 5, 15 min)"
      >
        <Activity className="size-3" />
        Load {s.load.map((l) => l.toFixed(2)).join(" ")}
      </span>
      <span className="flex shrink-0 items-center gap-1" title="Uptime">
        <Timer className="size-3" /> {formatUptime(s.uptimeSecs)}
      </span>
    </>
  );
}

function Meter({
  icon,
  label,
  pct,
  text,
  title,
}: {
  icon: React.ReactNode;
  label: string;
  pct: number | null;
  text: string;
  title: string;
}) {
  const lv = level(pct ?? 0);
  return (
    <span className="flex shrink-0 items-center gap-1.5" title={title}>
      {icon}
      <span>{label}</span>
      <span aria-hidden className="bg-muted relative h-1.5 w-12 overflow-hidden rounded-full">
        <span
          className={cn("absolute inset-y-0 left-0 rounded-full transition-[width]", LEVEL_BAR[lv])}
          style={{ width: `${String(pct ?? 0)}%` }}
        />
      </span>
      <span className={LEVEL_TEXT[lv]}>{text}</span>
    </span>
  );
}
