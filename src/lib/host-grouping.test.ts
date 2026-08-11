import { describe, expect, it } from "vitest";
import { baseDomain, groupByBaseDomain } from "@/lib/host-grouping";

describe("baseDomain", () => {
  it("collapses subdomains onto the registrable domain", () => {
    // The case from the report: five rows of one bank app.
    expect(baseDomain("acdn.t-bank-app.ru")).toBe("t-bank-app.ru");
    expect(baseDomain("api.t-bank-app.ru")).toBe("t-bank-app.ru");
    expect(baseDomain("cqohtp.inapps.appsflyersdk.com")).toBe("appsflyersdk.com");
    expect(baseDomain("mymts.apm-proxy.obs.mts.ru")).toBe("mts.ru");
  });

  it("keeps a bare domain as itself", () => {
    expect(baseDomain("mts.ru")).toBe("mts.ru");
    expect(baseDomain("localhost")).toBe("localhost");
  });

  it("keeps the same app on different TLDs apart", () => {
    // `.ru` and `.su` in the report are separate registrable domains and must
    // not be merged just because the second-level label matches.
    expect(baseDomain("api.t-bank-app.ru")).not.toBe(baseDomain("api.t-bank-app.su"));
  });

  it("takes a third label under a multi-label public suffix", () => {
    expect(baseDomain("api.example.co.uk")).toBe("example.co.uk");
    expect(baseDomain("cdn.shop.com.br")).toBe("shop.com.br");
  });

  it("normalises case and a trailing dot", () => {
    expect(baseDomain("API.T-Bank-App.RU.")).toBe("t-bank-app.ru");
  });
});

describe("groupByBaseDomain", () => {
  const hosts = [
    "acdn.t-bank-app.ru",
    "api.t-bank-app.ru",
    "api.t-bank-app.su",
    "cqohtp.inapps.appsflyersdk.com",
    "cqohtp.launches.appsflyersdk.com",
    "mymts.apm-proxy.obs.mts.ru",
  ].map((host) => ({ host }));

  it("buckets hosts under their domain, biggest group first", () => {
    const groups = groupByBaseDomain(hosts, (h) => h.host);
    expect(groups.map((g) => [g.domain, g.items.length])).toEqual([
      ["appsflyersdk.com", 2],
      ["t-bank-app.ru", 2],
      ["mts.ru", 1],
      ["t-bank-app.su", 1],
    ]);
  });

  it("loses nothing", () => {
    const groups = groupByBaseDomain(hosts, (h) => h.host);
    expect(groups.flatMap((g) => g.items.map((i) => i.host)).sort()).toEqual(
      hosts.map((h) => h.host).sort(),
    );
  });

  it("handles an empty list", () => {
    expect(groupByBaseDomain([], (h: { host: string }) => h.host)).toEqual([]);
  });
});
