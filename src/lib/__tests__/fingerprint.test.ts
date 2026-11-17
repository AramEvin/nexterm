import { describe, expect, it } from "vitest";

import { sha256Hex } from "@/lib/fingerprint";

describe("sha256Hex", () => {
  it("matches openssl's colon-separated hex", () => {
    // Same certificate: NexTerm shows base64, openssl shows hex.
    expect(sha256Hex("SHA256:TaNJ2lZeZxZHRvyslglCdhiFWDq0YdRposK1ou/nFkU")).toBe(
      "4D:A3:49:DA:56:5E:67:16:47:46:FC:AC:96:09:42:76:18:85:58:3A:B4:61:D4:69:A2:C2:B5:A2:EF:E7:16:45",
    );
  });

  it("rejects anything else", () => {
    expect(sha256Hex("MD5:abc")).toBeNull();
    expect(sha256Hex("SHA256:short")).toBeNull();
    expect(sha256Hex("SHA256:!!!")).toBeNull();
  });
});
