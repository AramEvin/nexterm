/**
 * TerminalController: owns one xterm.js instance for one tab.
 *
 * It lives outside React so the terminal survives re-renders and tab
 * switches, and so output that arrives before the view mounts (login
 * banners) is buffered by xterm itself. A controller outlives individual
 * sessions: on reconnect the same terminal (and its scrollback) is attached
 * to the new session id.
 */
import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal, type ITheme } from "@xterm/xterm";

import { sshResize, sshWrite } from "@/lib/ipc";
import { analyzePaste, stripControlChars } from "@/lib/paste";

const THEME: ITheme = {
  background: "#11141a",
  foreground: "#e3e6eb",
  cursor: "#5eead4",
  cursorAccent: "#11141a",
  selectionBackground: "#5eead455",
  black: "#1c2027",
  red: "#ff6b6b",
  green: "#6bd88f",
  yellow: "#f5c451",
  blue: "#6aa8ff",
  magenta: "#c792ea",
  cyan: "#5eead4",
  white: "#d5d9e0",
  brightBlack: "#5c6370",
  brightRed: "#ff8787",
  brightGreen: "#8fe6a8",
  brightYellow: "#ffd479",
  brightBlue: "#8cbcff",
  brightMagenta: "#dcb0f5",
  brightCyan: "#8ff3e1",
  brightWhite: "#ffffff",
};

/** Initial PTY size before the view has measured itself. */
export const DEFAULT_COLS = 120;
export const DEFAULT_ROWS = 32;

export type RendererKind = "webgl" | "dom";

/**
 * WebGL only in WebView2 (Windows). WebKitGTK (Linux) doesn't present the
 * WebGL canvas of a terminal without focus until the next input, so output in
 * a split pane or behind the side panel looked frozen; the DOM renderer has
 * no such problem there.
 */
const WEBGL_SUPPORTED = typeof navigator !== "undefined" && /Windows/.test(navigator.userAgent);

/** Input the user typed or pasted (not terminal replies), for multi-exec. */
export type UserInput = { kind: "key"; data: string } | { kind: "paste"; text: string };

/**
 * Decides what a paste becomes: the (possibly edited) text to send, or
 * `null` to cancel. Set by the app to show the paste safety check.
 */
export type PasteGuard = (text: string) => Promise<string | null>;

export class TerminalController {
  readonly term: Terminal;
  private readonly fit = new FitAddon();
  private sessionId: string | null = null;
  private resizeObserver: ResizeObserver | null = null;
  private element: HTMLElement | null = null;
  private webgl: WebglAddon | null = null;
  private visible = false;
  private disposed = false;
  private fitFrame = 0;
  /** Input typed while a write is in flight; sent as one batch afterwards. */
  private pendingInput = "";
  private writing = false;
  private readonly onContextMenu = (e: MouseEvent) => {
    e.preventDefault();
    void this.copyOrPaste();
  };
  /**
   * Every paste (Ctrl+Shift+V, Shift+Insert, menu) goes through
   * `handlePaste` instead of straight into xterm, so it can be reviewed.
   * Capture phase on the container runs before xterm's own listener.
   */
  private readonly onPaste = (e: ClipboardEvent) => {
    e.preventDefault();
    e.stopImmediatePropagation();
    void this.handlePaste(e.clipboardData?.getData("text/plain") ?? "");
  };
  renderer: RendererKind = "dom";
  /** Called with keys and pastes the user sends here (multi-exec fan-out). */
  onUserInput: ((input: UserInput) => void) | null = null;
  /** Paste safety check; without one, control characters are still stripped. */
  pasteGuard: PasteGuard | null = null;

  constructor() {
    this.term = new Terminal({
      allowProposedApi: true, // required by the unicode11 addon
      cursorBlink: true,
      fontFamily: '"Cascadia Mono", "Cascadia Code", Consolas, "DejaVu Sans Mono", monospace',
      fontSize: 14,
      scrollback: 10_000,
      theme: THEME,
    });
    this.term.loadAddon(this.fit);
    this.term.loadAddon(new Unicode11Addon());
    this.term.unicode.activeVersion = "11"; // correct widths for emoji / CJK
    this.term.attachCustomKeyEventHandler((e) => this.handleKey(e));

    // Registered once; they act on whichever session is attached right now.
    this.term.onData((data) => {
      this.queueInput(data);
    });
    // onData also carries the terminal's automatic replies (cursor reports,
    // focus events); only real keystrokes are fanned out to other terminals.
    this.term.onKey(({ key }) => {
      if (this.sessionId) this.onUserInput?.({ kind: "key", data: key });
    });
    this.term.onResize(({ cols, rows }) => {
      if (this.sessionId) void sshResize(this.sessionId, cols, rows).catch(() => undefined);
    });
  }

  /** Current size, used when (re)connecting so the PTY starts at the right size. */
  get size(): { cols: number; rows: number } {
    return this.element
      ? { cols: this.term.cols, rows: this.term.rows }
      : { cols: DEFAULT_COLS, rows: DEFAULT_ROWS };
  }

  /** Connect this terminal to a live backend session (first connect or reconnect). */
  attachSession(sessionId: string): void {
    this.sessionId = sessionId;
    this.pendingInput = "";
    this.term.options.disableStdin = false;
    this.term.options.cursorBlink = true;
    // Sync the real size now that the backend can receive it.
    if (this.element) {
      this.scheduleFit();
      void sshResize(sessionId, this.term.cols, this.term.rows).catch(() => undefined);
    }
  }

  /** Stop sending input (the session ended) but keep the scrollback visible. */
  detachSession(): void {
    this.sessionId = null;
    this.pendingInput = "";
    this.term.options.disableStdin = true;
    this.term.options.cursorBlink = false;
  }

  /** Mount into a DOM element. Called once per controller. */
  open(element: HTMLElement): void {
    if (this.element || this.disposed) return;
    this.element = element;
    this.term.open(element);
    element.addEventListener("contextmenu", this.onContextMenu);
    element.addEventListener("paste", this.onPaste, { capture: true });

    this.resizeObserver = new ResizeObserver(() => {
      this.scheduleFit();
    });
    this.resizeObserver.observe(element);
    this.syncRenderer();
    this.scheduleFit();
  }

  /**
   * Chromium allows only ~16 live WebGL contexts per page, so with many tabs
   * only the visible terminal holds one. Hidden tabs fall back to the DOM
   * renderer, which costs nothing while not painting.
   */
  setVisible(visible: boolean): void {
    this.visible = visible;
    this.syncRenderer();
    if (visible) this.scheduleFit();
  }

  focus(): void {
    this.term.focus();
  }

  /** Re-measure after the pane's size changed (split, resize bar). */
  refit(): void {
    if (this.visible) this.scheduleFit();
  }

  /** Whether a live session is attached (input is accepted). */
  get attached(): boolean {
    return this.sessionId !== null;
  }

  /** Multi-exec: a key typed in another terminal. */
  sendKey(data: string): void {
    this.queueInput(data);
  }

  /** Multi-exec: a paste already reviewed in another terminal. */
  pasteReviewed(text: string): void {
    if (this.sessionId) this.term.paste(text);
  }

  /** Paste text after the safety check (used by the menu and keyboard paste). */
  async handlePaste(text: string): Promise<void> {
    if (!this.sessionId || text === "") return;
    let final: string | null;
    if (this.pasteGuard) {
      final = await this.pasteGuard(text);
    } else {
      final = analyzePaste(text).controlChars > 0 ? stripControlChars(text) : text;
    }
    if (final !== null && final !== "" && this.sessionId) {
      this.term.paste(final);
      this.onUserInput?.({ kind: "paste", text: final });
    }
    this.term.focus();
  }

  /** Print a dim NexTerm notice line into the terminal. */
  notice(text: string): void {
    this.term.write(`\r\n\x1b[2m[NexTerm] ${text}\x1b[0m\r\n`);
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.sessionId = null;
    cancelAnimationFrame(this.fitFrame);
    this.resizeObserver?.disconnect();
    this.element?.removeEventListener("contextmenu", this.onContextMenu);
    this.element?.removeEventListener("paste", this.onPaste, { capture: true });
    this.onUserInput = null;
    this.pasteGuard = null;
    this.disposeWebgl();
    this.term.dispose(); // also disposes the remaining addons
    this.element = null;
  }

  private syncRenderer(): void {
    if (!this.element || this.disposed) return;
    if (this.visible && !this.webgl && WEBGL_SUPPORTED) {
      try {
        const webgl = new WebglAddon();
        webgl.onContextLoss(() => {
          this.disposeWebgl(); // falls back to the DOM renderer
        });
        this.term.loadAddon(webgl);
        this.webgl = webgl;
        this.renderer = "webgl";
      } catch {
        this.renderer = "dom"; // WebGL unavailable (old GPU / RDP / VM)
      }
    } else if (!this.visible && this.webgl) {
      this.disposeWebgl();
    }
  }

  private disposeWebgl(): void {
    if (!this.webgl) return;
    const webgl = this.webgl;
    this.webgl = null;
    this.renderer = "dom";
    try {
      webgl.dispose();
    } catch {
      // already torn down with the terminal
    }
  }

  /**
   * IPC calls run concurrently on the Rust side, so firing one `ssh_write`
   * per keystroke can reorder fast typing. Keep exactly one write in flight
   * and batch everything typed meanwhile: order is preserved and a fast
   * typist or a paste costs a handful of IPC calls instead of hundreds.
   */
  private queueInput(data: string): void {
    if (!this.sessionId) return;
    this.pendingInput += data;
    if (!this.writing) void this.flushInput();
  }

  private async flushInput(): Promise<void> {
    this.writing = true;
    try {
      while (this.pendingInput && this.sessionId) {
        const chunk = this.pendingInput;
        this.pendingInput = "";
        await sshWrite(this.sessionId, chunk);
      }
    } catch {
      this.pendingInput = ""; // session closed; the status channel says why
    } finally {
      this.writing = false;
    }
  }

  /** Coalesce resize bursts (window drag) into one fit per animation frame. */
  private scheduleFit(): void {
    cancelAnimationFrame(this.fitFrame);
    this.fitFrame = requestAnimationFrame(() => {
      const el = this.term.element;
      // Hidden tabs have zero size; fitting then would shrink the PTY to 1x1.
      if (this.disposed || !el || el.clientWidth === 0 || el.clientHeight === 0) return;
      this.fit.fit();
    });
  }

  /**
   * Ctrl+Shift+C copies the selection. Ctrl+Shift+V and Shift+Insert are
   * left to the browser's native paste event, which `onPaste` catches for the
   * safety check before xterm sends it (wrapped in bracketed-paste markers
   * when the remote app asks for it).
   */
  private handleKey(e: KeyboardEvent): boolean {
    if (e.type !== "keydown") return true;
    if (e.ctrlKey && e.shiftKey && e.code === "KeyC") {
      const selection = this.term.getSelection();
      if (selection) void navigator.clipboard.writeText(selection);
      e.preventDefault();
      return false;
    }
    if ((e.ctrlKey && e.shiftKey && e.code === "KeyV") || (e.shiftKey && e.code === "Insert")) {
      return false; // let the native paste event reach xterm's textarea
    }
    return true;
  }

  /** Right-click: copy if text is selected, otherwise paste. */
  private async copyOrPaste(): Promise<void> {
    const selection = this.term.getSelection();
    try {
      if (selection) {
        await navigator.clipboard.writeText(selection);
        this.term.clearSelection();
      } else if (this.sessionId) {
        const text = await navigator.clipboard.readText();
        await this.handlePaste(text);
        return;
      }
    } catch {
      // Clipboard permission denied by the webview; keyboard shortcuts still work.
    }
    this.term.focus();
  }
}
