/** Mirrors `vault::MIN_MASTER_PASSWORD_LEN` (Rust enforces it too). */
export const MIN_MASTER_PASSWORD_LENGTH = 8;

/**
 * Rough guidance for the master password. The real protection is Argon2id
 * plus length; this only nudges people toward long passphrases.
 */
export function passwordStrength(pw: string): { score: 0 | 1 | 2 | 3; label: string } {
  if (pw.length < MIN_MASTER_PASSWORD_LENGTH) {
    return { score: 0, label: `At least ${String(MIN_MASTER_PASSWORD_LENGTH)} characters` };
  }
  const classes = [/[a-z]/, /[A-Z]/, /\d/, /[^A-Za-z0-9]/].filter((r) => r.test(pw)).length;
  if (pw.length >= 16 || (pw.length >= 12 && classes >= 3)) return { score: 3, label: "Strong" };
  if (pw.length >= 12 || classes >= 3) return { score: 2, label: "OK" };
  return { score: 1, label: "Weak: use 12+ characters or a passphrase" };
}
