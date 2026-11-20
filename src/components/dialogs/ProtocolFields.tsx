import { useEffect, useState } from "react";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { localShells } from "@/lib/ipc";
import type {
  FlowControl,
  LocalOptions,
  LocalShellInfo,
  Parity,
  SerialOptions,
} from "@/types/library";

export const selectClass =
  "border-input bg-background dark:bg-input/30 h-9 w-full rounded-md border px-3 text-sm outline-none focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]";

const BAUD_RATES = [
  300, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600,
];

/** Port name and line settings of a serial session. */
export function SerialFields({
  value,
  onChange,
}: {
  value: SerialOptions;
  onChange: (v: SerialOptions) => void;
}) {
  const set = (patch: Partial<SerialOptions>) => {
    onChange({ ...value, ...patch });
  };
  return (
    <fieldset className="grid gap-3 rounded-md border p-3">
      <legend className="px-1 text-sm font-medium">Serial line</legend>
      <div className="grid grid-cols-[1fr_9rem] gap-3">
        <div className="grid gap-1.5">
          <Label htmlFor="se-serial-port">Port</Label>
          <Input
            id="se-serial-port"
            placeholder="COM3 (or /dev/ttyUSB0)"
            value={value.port}
            spellCheck={false}
            maxLength={64}
            onChange={(e) => {
              set({ port: e.target.value });
            }}
          />
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="se-baud">Speed (baud)</Label>
          <select
            id="se-baud"
            className={selectClass}
            value={value.baudRate}
            onChange={(e) => {
              set({ baudRate: Number(e.target.value) });
            }}
          >
            {(BAUD_RATES.includes(value.baudRate)
              ? BAUD_RATES
              : [value.baudRate, ...BAUD_RATES]
            ).map((b) => (
              <option key={b} value={b}>
                {b.toLocaleString()}
              </option>
            ))}
          </select>
        </div>
      </div>
      <div className="grid grid-cols-4 gap-3">
        <div className="grid gap-1.5">
          <Label htmlFor="se-databits">Data bits</Label>
          <select
            id="se-databits"
            className={selectClass}
            value={value.dataBits}
            onChange={(e) => {
              set({ dataBits: Number(e.target.value) });
            }}
          >
            {[8, 7, 6, 5].map((b) => (
              <option key={b} value={b}>
                {b}
              </option>
            ))}
          </select>
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="se-parity">Parity</Label>
          <select
            id="se-parity"
            className={selectClass}
            value={value.parity}
            onChange={(e) => {
              set({ parity: e.target.value as Parity });
            }}
          >
            <option value="none">None</option>
            <option value="even">Even</option>
            <option value="odd">Odd</option>
          </select>
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="se-stopbits">Stop bits</Label>
          <select
            id="se-stopbits"
            className={selectClass}
            value={value.stopBits}
            onChange={(e) => {
              set({ stopBits: Number(e.target.value) });
            }}
          >
            <option value={1}>1</option>
            <option value={2}>2</option>
          </select>
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="se-flow">Flow control</Label>
          <select
            id="se-flow"
            className={selectClass}
            value={value.flowControl}
            onChange={(e) => {
              set({ flowControl: e.target.value as FlowControl });
            }}
          >
            <option value="none">None</option>
            <option value="software">XON/XOFF</option>
            <option value="hardware">RTS/CTS</option>
          </select>
        </div>
      </div>
      <p className="text-muted-foreground text-xs">
        The port name is in Device Manager under &quot;Ports (COM &amp; LPT)&quot;. Most network
        gear uses 9600 8N1; newer devices often 115200.
      </p>
    </fieldset>
  );
}

/** Shell choice of a local-terminal session. */
export function LocalFields({
  value,
  onChange,
}: {
  value: LocalOptions;
  onChange: (v: LocalOptions) => void;
}) {
  const [shells, setShells] = useState<LocalShellInfo[]>([]);
  useEffect(() => {
    let live = true;
    void localShells()
      .then((list) => {
        if (live) setShells(list);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, []);
  // A shell this PC doesn't offer (e.g. a session made on Windows, edited on Linux) stays listed.
  const known = shells.some((s) => s.shell === value.shell);
  return (
    <fieldset className="grid gap-3 rounded-md border p-3">
      <legend className="px-1 text-sm font-medium">Local terminal</legend>
      <div className="grid grid-cols-2 gap-3">
        <div className="grid gap-1.5">
          <Label htmlFor="se-shell">Shell</Label>
          <select
            id="se-shell"
            className={selectClass}
            value={value.shell}
            onChange={(e) => {
              const shell = e.target.value as LocalOptions["shell"];
              onChange({ shell, wslDistro: shell === "wsl" ? value.wslDistro : "" });
            }}
          >
            {!known && <option value={value.shell}>{value.shell}</option>}
            {shells.map((s) => (
              <option key={s.shell} value={s.shell} disabled={!s.available}>
                {s.label}
                {s.available ? "" : " (not installed)"}
              </option>
            ))}
          </select>
        </div>
        {value.shell === "wsl" && (
          <div className="grid gap-1.5">
            <Label htmlFor="se-distro">Distribution</Label>
            <Input
              id="se-distro"
              placeholder="Default"
              value={value.wslDistro}
              maxLength={64}
              spellCheck={false}
              onChange={(e) => {
                onChange({ ...value, wslDistro: e.target.value });
              }}
            />
          </div>
        )}
      </div>
      <p className="text-muted-foreground text-xs">
        Runs on this computer, starting in your home folder. Handy to keep next to your servers.
      </p>
    </fieldset>
  );
}
