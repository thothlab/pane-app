import { describe, expect, it } from "vitest";
import {
  FORMAT_ID,
  FORMAT_VERSION,
  looksLikeReadableDump,
  parseImportFile,
  readableDumpToPortFile,
  validatePortFile,
} from "@/lib/rules-portfile";

// A rules file that came from a fork — same app, but its exporter
// serialises the backend DTOs directly instead of building a port file.
// It used to be rejected with "unexpected format: undefined", which reads
// as "your file is corrupt" when the file is perfectly good, just a
// different revision of the shape.
const readableDump = {
  source: "pane rules export",
  exported_at: "2026-08-10 12:15:36.226995 +00:00:00",
  collections: [
    {
      name: "база 500 ₽",
      enabled: false,
      rule_count: 1,
      rules: [
        {
          name: "УПК·base· scan 1.1",
          enabled: true,
          mode: "stub",
          match: {
            method: "POST",
            path: "/b/qr-adapter/1.1/getPaymentsByQRcode",
            host: null,
            req_body: { QRcode: "aHR0cHM6" },
            params: [],
            conditions: [],
          },
          response: {
            status: 200,
            delay_ms: 1200,
            mime: "application/json;charset=UTF-8",
            body: { requestId: "r-base", errorCode: 0 },
          },
          patches: [],
        },
      ],
    },
  ],
};

function decode(b64: string): string {
  const bin = atob(b64);
  const bytes = Uint8Array.from(bin, (ch) => ch.charCodeAt(0));
  return new TextDecoder().decode(bytes);
}

describe("parseImportFile", () => {
  it("accepts our own port file unchanged", () => {
    const file = {
      format: FORMAT_ID,
      version: FORMAT_VERSION,
      exported_at: "2026-08-10T12:15:36.226Z",
      kind: "library",
      collections: [],
      rules: [],
    };
    expect(parseImportFile(file)).toBe(file);
  });

  it("accepts a readable dump from a fork", () => {
    const out = parseImportFile(readableDump);
    expect(typeof out).not.toBe("string");
    const file = out as Exclude<typeof out, string>;
    expect(file.format).toBe(FORMAT_ID);
    expect(file.kind).toBe("library");
    expect(file.collections).toHaveLength(1);
    expect(file.rules).toHaveLength(1);
  });

  it("explains a missing format instead of blaming the file", () => {
    // The old message was "unexpected format: undefined".
    const msg = parseImportFile({ hello: "world" });
    expect(typeof msg).toBe("string");
    expect(msg as string).toContain("no `format` field");
  });

  it("still rejects a foreign format by name", () => {
    expect(parseImportFile({ format: "charles-session", rules: [] })).toContain(
      "unexpected format: charles-session",
    );
  });

  it("rejects non-objects", () => {
    expect(parseImportFile(null)).toBe("not a JSON object");
    expect(parseImportFile("[]")).toBe("not a JSON object");
  });
});

describe("readableDumpToPortFile", () => {
  const file = readableDumpToPortFile(readableDump) as Exclude<
    ReturnType<typeof readableDumpToPortFile>,
    string
  >;
  const rule = file.rules[0];

  it("flattens match/response into the port file's fields", () => {
    expect(rule.match_method).toBe("POST");
    expect(rule.match_path_glob).toBe("/b/qr-adapter/1.1/getPaymentsByQRcode");
    expect(rule.match_host_glob).toBeNull();
    expect(rule.res_status).toBe(200);
    expect(rule.res_delay_ms).toBe(1200);
    expect(rule.res_body_mime).toBe("application/json;charset=UTF-8");
  });

  it("stringifies req_body, which the backend takes as JSON text", () => {
    expect(typeof rule.match_req_body).toBe("string");
    expect(JSON.parse(rule.match_req_body!)).toEqual({ QRcode: "aHR0cHM6" });
  });

  it("encodes the literal body as base64 without changing it", () => {
    expect(JSON.parse(decode(rule.res_body_base64!))).toEqual({
      requestId: "r-base",
      errorCode: 0,
    });
  });

  it("round-trips non-ASCII bodies, which btoa alone would throw on", () => {
    const dump = {
      collections: [
        {
          name: "к",
          rules: [
            {
              name: "к",
              match: { method: "GET", path: "/x" },
              response: { body: { сообщение: "привет 🙂" } },
            },
          ],
        },
      ],
    };
    const out = readableDumpToPortFile(dump) as Exclude<
      ReturnType<typeof readableDumpToPortFile>,
      string
    >;
    expect(JSON.parse(decode(out.rules[0].res_body_base64!))).toEqual({
      сообщение: "привет 🙂",
    });
  });

  it("derives priority from array order, which is evaluation order", () => {
    const dump = {
      collections: [
        { name: "a", rules: [{ name: "a1", match: {} }, { name: "a2", match: {} }] },
        { name: "b", rules: [{ name: "b1", match: {} }] },
      ],
    };
    const out = readableDumpToPortFile(dump) as Exclude<
      ReturnType<typeof readableDumpToPortFile>,
      string
    >;
    expect(out.collections!.map((c) => c.priority)).toEqual([0, 1]);
    expect(out.rules.map((r) => [r.name, r.priority])).toEqual([
      ["a1", 0],
      ["a2", 1],
      ["b1", 0],
    ]);
    expect(out.rules[2].collection_ref).toBe(out.collections![1].ref);
  });

  it("treats a null req_body as no body matching", () => {
    const out = readableDumpToPortFile({
      collections: [
        { name: "c", rules: [{ name: "r", match: { req_body: null } }] },
      ],
    }) as Exclude<ReturnType<typeof readableDumpToPortFile>, string>;
    expect(out.rules[0].match_req_body).toBeNull();
  });

  it("keeps an already-encoded body instead of double-encoding it", () => {
    const out = readableDumpToPortFile({
      collections: [
        {
          name: "c",
          rules: [
            { name: "r", match: {}, response: { body_base64: "aGk=" } },
          ],
        },
      ],
    }) as Exclude<ReturnType<typeof readableDumpToPortFile>, string>;
    expect(decode(out.rules[0].res_body_base64!)).toBe("hi");
  });

  it("puts top-level rules in the ungrouped bucket", () => {
    const out = readableDumpToPortFile({
      rules: [{ name: "loose", match: { method: "GET", path: "/x" } }],
    }) as Exclude<ReturnType<typeof readableDumpToPortFile>, string>;
    expect(out.rules[0].collection_ref).toBeNull();
  });

  it("reports an empty file rather than importing nothing silently", () => {
    expect(readableDumpToPortFile({ collections: [] })).toBe(
      "no rules found in this file",
    );
  });
});

describe("looksLikeReadableDump", () => {
  it("never claims a port file", () => {
    expect(
      looksLikeReadableDump({ format: FORMAT_ID, version: 1, rules: [] }),
    ).toBe(false);
  });

  it("recognises nested collection rules", () => {
    expect(looksLikeReadableDump(readableDump)).toBe(true);
  });

  it("ignores a bare object with no rules anywhere", () => {
    expect(looksLikeReadableDump({ collections: [] })).toBe(false);
  });
});

describe("validatePortFile", () => {
  // Kept as the strict gate for files that do declare our format —
  // parseImportFile delegates to it.
  it("rejects a version newer than we understand", () => {
    expect(
      validatePortFile({
        format: FORMAT_ID,
        version: FORMAT_VERSION + 1,
        kind: "library",
        rules: [],
      }),
    ).toContain("is newer than supported");
  });

  it("rejects an unknown kind", () => {
    expect(
      validatePortFile({
        format: FORMAT_ID,
        version: 1,
        kind: "everything",
        rules: [],
      }),
    ).toContain("unknown kind");
  });
});
