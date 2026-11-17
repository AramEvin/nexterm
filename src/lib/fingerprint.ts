/**
 * "SHA256:<base64>" -> "4D:A3:49:..." (the hex form `openssl x509 -fingerprint
 * -sha256` prints), or null if the input isn't a SHA-256 fingerprint.
 */
export function sha256Hex(fingerprint: string): string | null {
  const b64 = fingerprint.startsWith("SHA256:") ? fingerprint.slice(7) : null;
  if (!b64 || !/^[A-Za-z0-9+/]+=*$/.test(b64)) return null;
  const padded = b64 + "=".repeat((4 - (b64.length % 4)) % 4);
  let bytes: string;
  try {
    bytes = atob(padded);
  } catch {
    return null;
  }
  if (bytes.length !== 32) return null;
  return Array.from(bytes, (c) => c.charCodeAt(0).toString(16).padStart(2, "0").toUpperCase()).join(
    ":",
  );
}
