import { describe, expect, it } from "vitest";
import {
  matchesCollectionName,
  matchesRuleFilter,
  parseFilterTerms,
  ruleHaystack,
} from "./rules-filter";
import type { RuleDto } from "@/ipc/types";

function rule(over: Partial<RuleDto> = {}): RuleDto {
  return {
    id: "r1",
    name: "Orders empty list",
    enabled: true,
    priority: 0,
    collection_id: "c1",
    mode: "stub",
    patches: [],
    match_host_glob: "api.example.com",
    match_method: "GET",
    match_path_glob: "/v2/orders",
    match_params: [],
    match_req_body: null,
    match_conditions: [],
    res_status: 200,
    res_headers: [],
    res_body_id: null,
    res_body_mime: null,
    res_body_size: 0,
    res_delay_ms: 0,
    created_at: "",
    updated_at: "",
    ...over,
  } as RuleDto;
}

describe("parseFilterTerms", () => {
  it("returns no terms for empty or blank input", () => {
    expect(parseFilterTerms("")).toEqual([]);
    expect(parseFilterTerms("   ")).toEqual([]);
  });

  it("splits on any run of whitespace and lowercases", () => {
    expect(parseFilterTerms("  Orders   500 ")).toEqual(["orders", "500"]);
  });
});

describe("matchesRuleFilter", () => {
  const terms = parseFilterTerms;

  it("matches everything when the query is empty", () => {
    expect(matchesRuleFilter(rule(), "Checkout", terms(""))).toBe(true);
  });

  it("matches on the rule name, case-insensitively", () => {
    expect(matchesRuleFilter(rule(), "Checkout", terms("ORDERS"))).toBe(true);
  });

  it("matches on the collection name — the group-name case", () => {
    expect(matchesRuleFilter(rule(), "Checkout flow", terms("checkout"))).toBe(true);
  });

  it("matches on host, path and method of the request", () => {
    const r = rule({ name: "nothing relevant here" });
    expect(matchesRuleFilter(r, "Grp", terms("api.example.com"))).toBe(true);
    expect(matchesRuleFilter(r, "Grp", terms("/v2/orders"))).toBe(true);
    expect(matchesRuleFilter(r, "Grp", terms("get"))).toBe(true);
  });

  it("matches on query params, name or value", () => {
    const r = rule({
      name: "x",
      match_path_glob: "/x",
      match_params: [{ name: "status", value: "shipped" }],
    });
    expect(matchesRuleFilter(r, "Grp", terms("shipped"))).toBe(true);
    expect(matchesRuleFilter(r, "Grp", terms("status=shipped"))).toBe(true);
  });

  it("ANDs several terms — all must match, order irrelevant", () => {
    const r = rule({ name: "Orders 500" });
    expect(matchesRuleFilter(r, "Checkout", terms("orders 500"))).toBe(true);
    expect(matchesRuleFilter(r, "Checkout", terms("500 orders"))).toBe(true);
    expect(matchesRuleFilter(r, "Checkout", terms("orders 404"))).toBe(false);
  });

  it("ANDs across different fields — one term from the name, one from the host", () => {
    expect(matchesRuleFilter(rule(), "Checkout", terms("orders api.example"))).toBe(
      true,
    );
  });

  it("does not match on the response side (status / body stay out)", () => {
    const r = rule({ name: "x", match_host_glob: "h", match_path_glob: "/p" });
    expect(matchesRuleFilter(r, "Grp", terms("200"))).toBe(false);
  });

  it("survives null matchers instead of throwing", () => {
    const r = rule({
      name: "catch all",
      match_host_glob: null,
      match_method: null,
      match_path_glob: null,
    });
    expect(matchesRuleFilter(r, "Grp", terms("catch"))).toBe(true);
    expect(matchesRuleFilter(r, "Grp", terms("null"))).toBe(false);
  });
});

describe("ruleHaystack", () => {
  it("is lowercased so callers never have to normalise twice", () => {
    expect(ruleHaystack(rule(), "Checkout")).toBe(
      "orders empty list checkout get api.example.com /v2/orders",
    );
  });
});

describe("matchesCollectionName", () => {
  it("keeps an empty group visible when its name is typed", () => {
    expect(matchesCollectionName("Checkout flow", parseFilterTerms("checkout"))).toBe(
      true,
    );
    expect(matchesCollectionName("Checkout flow", parseFilterTerms("orders"))).toBe(
      false,
    );
  });

  it("matches everything when the query is empty", () => {
    expect(matchesCollectionName("Anything", parseFilterTerms(""))).toBe(true);
  });
});
