import { describe, expect, it } from "vitest";
import {
  redactHeaders,
  redactHeaderValue,
  SECRET_PLACEHOLDER as T,
} from "@/lib/redact";

describe("redactHeaderValue", () => {
  it("keeps the auth scheme and hides the credential", () => {
    expect(redactHeaderValue("Authorization", "Bearer v2.local.i4WFjFqu")).toBe(
      `Bearer ${T}`,
    );
    expect(redactHeaderValue("proxy-authorization", "Basic dXNlcjpwYXNz")).toBe(
      `Basic ${T}`,
    );
  });

  it("masks a bare authorization value whole", () => {
    expect(redactHeaderValue("AUTHORIZATION", "abc123")).toBe(T);
  });

  it("keeps cookie names, hides their values", () => {
    expect(redactHeaderValue("Cookie", "a=1;b=xy=z; c=3")).toBe(
      `a=${T};b=${T}; c=${T}`,
    );
  });

  it("keeps Set-Cookie attributes", () => {
    expect(
      redactHeaderValue("set-cookie", "sid=abc; Path=/; HttpOnly; Secure"),
    ).toBe(`sid=${T}; Path=/; HttpOnly; Secure`);
  });

  it("masks custom token-like headers", () => {
    for (const name of ["X-Access-Token", "X-Api-Key", "api_key", "X-Session-Id", "X-CSRF-Token", "Client-Secret", "X-Auth", "X-Auth-User"]) {
      expect(redactHeaderValue(name, "s3cr3t")).toBe(T);
    }
  });

  it("leaves identifiers and ordinary headers alone", () => {
    for (const name of ["ClientID", "ProfileId", "Request-id", "Tid", "Phone", "Content-Type", "baggage", "sentry-trace", "WWW-Authenticate", "Proxy-Authenticate", "Author"]) {
      expect(redactHeaderValue(name, "value-1")).toBe("value-1");
    }
  });

  it("leaves an empty secret empty", () => {
    expect(redactHeaderValue("Authorization", "")).toBe("");
  });
});

describe("redactHeaders", () => {
  it("counts only headers it changed and keeps order and names", () => {
    const { headers, masked } = redactHeaders([
      { name: "Host", value: "api.example.com" },
      { name: "Authorization", value: "Bearer x" },
      { name: "Cookie", value: "a=1" },
      { name: "Authorization", value: "" },
    ]);
    expect(masked).toBe(2);
    expect(headers).toEqual([
      { name: "Host", value: "api.example.com" },
      { name: "Authorization", value: `Bearer ${T}` },
      { name: "Cookie", value: `a=${T}` },
      { name: "Authorization", value: "" },
    ]);
  });
});
