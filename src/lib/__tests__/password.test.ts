import { describe, expect, it } from "vitest";

import { passwordStrength } from "@/lib/password";

describe("passwordStrength", () => {
  it("rates by length and variety", () => {
    expect(passwordStrength("short").score).toBe(0);
    expect(passwordStrength("abcdefgh").score).toBe(1);
    expect(passwordStrength("abcdefghijkl").score).toBe(2);
    expect(passwordStrength("Abcdefgh1!xy").score).toBe(3);
    expect(passwordStrength("correct horse battery staple").score).toBe(3);
  });
});
