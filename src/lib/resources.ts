/** Formatting for the resource bar. Pure functions (unit-tested). */

/** `kib` → "1.5 GB" style text (binary units, shown with the usual short names). */
export function formatKib(kib: number): string {
  if (!Number.isFinite(kib) || kib < 0) return "–";
  const units = ["KB", "MB", "GB", "TB", "PB"];
  let value = kib;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = value >= 100 || unit === 0 ? 0 : 1;
  return `${value.toFixed(digits)} ${units[unit] ?? "KB"}`;
}

/** Seconds → "3d 4h", "5h 12m", "7m". */
export function formatUptime(secs: number): string {
  if (!Number.isFinite(secs) || secs < 0) return "–";
  const d = Math.floor(secs / 86_400);
  const h = Math.floor((secs % 86_400) / 3_600);
  const m = Math.floor((secs % 3_600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

export function percent(used: number, total: number): number {
  if (!(total > 0) || !Number.isFinite(used)) return 0;
  return Math.min(100, Math.max(0, (used / total) * 100));
}

/** Disk use the way `df` shows it: used / (used + available). */
export function diskPercent(usedKib: number, availKib: number): number {
  return percent(usedKib, usedKib + availKib);
}

export type Level = "ok" | "warn" | "critical";

/** Thresholds for the colour of a meter (always shown with its number too). */
export function level(pct: number): Level {
  if (pct >= 90) return "critical";
  if (pct >= 75) return "warn";
  return "ok";
}

/** Load relative to cores: 1.0 per core = fully busy. */
export function loadLevel(load1: number, cores: number): Level {
  return level(percent(load1, Math.max(1, cores)));
}
