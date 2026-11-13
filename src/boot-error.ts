/**
 * Loaded before the app bundle. If startup throws, show the error instead of
 * a blank window, so users can report something useful. No inline script
 * needed (CSP forbids it).
 */
function show(message: string): void {
  const root = document.getElementById("root");
  if (!root || root.childElementCount > 0) return; // app already rendered
  const box = document.createElement("pre");
  box.style.cssText =
    "margin:2rem;padding:1rem;border:1px solid #f87171;border-radius:8px;color:#fecaca;" +
    "background:#1f1315;white-space:pre-wrap;font:13px Consolas,monospace;user-select:text";
  box.textContent = `NexTerm failed to start.\n\n${message}`;
  root.appendChild(box);
}

window.addEventListener("error", (e) => {
  const err: unknown = e.error;
  show(err instanceof Error ? `${err.name}: ${err.message}\n${err.stack ?? ""}` : e.message);
});
window.addEventListener("unhandledrejection", (e) => {
  const reason: unknown = e.reason;
  show(reason instanceof Error ? `${reason.name}: ${reason.message}` : String(reason));
});
