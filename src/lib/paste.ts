/**
 * Paste safety check (Phase 6).
 *
 * A paste with a line break runs commands the moment it lands in a shell, and
 * text copied from a web page can hide characters the eye doesn't see
 * (escape sequences, a fake "end of bracketed paste" marker). Such pastes are
 * shown for review first.
 */

/** Pastes longer than this are reviewed even on one line. */
export const LARGE_PASTE_CHARS = 2000;

export interface PasteAnalysis {
  lines: number;
  chars: number;
  multiLine: boolean;
  large: boolean;
  /** Text ends with a line break: the last line would run immediately. */
  endsWithNewline: boolean;
  /** Invisible control characters (ESC, NUL, ...) found; they are removed. */
  controlChars: number;
  /** Short notes about commands that deserve a second look. */
  risks: string[];
  needsReview: boolean;
}

// C0 controls except TAB, LF, CR; DEL; C1 controls (0x80-0x9F).
// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F\u0080-\u009F]/g;

const RISKS: [RegExp, string][] = [
  [/\bsudo\b|\bsu\s+-/, "Runs commands as root (sudo / su)."],
  [/\brm\s+(-[a-zA-Z]*[rf][a-zA-Z]*\s+)+/, "Deletes files (rm -r / -f)."],
  [/\b(curl|wget)\b[^|\n]*\|\s*(sudo\s+)?(ba|z|da)?sh\b/, "Downloads and runs a script (… | sh)."],
  [/\bmkfs(\.\w+)?\b|\bdd\s+[^\n]*\bof=\/dev\//, "Writes to a disk device (mkfs / dd)."],
  [/\bchmod\s+(-R\s+)?777\b/, "Makes files writable by everyone (chmod 777)."],
  [/\b(shutdown|reboot|poweroff|halt)\b/, "Shuts down or restarts the machine."],
  [/:\(\)\s*\{\s*:\|:&\s*\};:/, "Fork bomb."],
];

export function analyzePaste(text: string): PasteAnalysis {
  const normalized = text.replace(/\r\n?/g, "\n");
  const body = normalized.endsWith("\n") ? normalized.slice(0, -1) : normalized;
  const lines = body === "" ? (normalized === "" ? 0 : 1) : body.split("\n").length;
  const controlChars = (text.match(CONTROL) ?? []).length;
  const multiLine = normalized.includes("\n");
  const large = text.length > LARGE_PASTE_CHARS;
  const risks = RISKS.filter(([re]) => re.test(normalized)).map(([, note]) => note);
  return {
    lines,
    chars: text.length,
    multiLine,
    large,
    endsWithNewline: normalized.endsWith("\n"),
    controlChars,
    risks,
    needsReview: multiLine || large || controlChars > 0,
  };
}

/** Remove invisible control characters (keeps TAB and line breaks). */
export function stripControlChars(text: string): string {
  return text.replace(CONTROL, "");
}

/** Remove trailing line breaks so the last line waits for Enter. */
export function withoutFinalNewline(text: string): string {
  return text.replace(/[\r\n]+$/, "");
}
