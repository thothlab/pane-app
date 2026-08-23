import { describe, expect, it } from "vitest";
import {
  matchesCollection,
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
    tags: [],
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
    expect(parseFilterTerms("  Orders   500 ")).toEqual([
      { key: "any", values: ["orders"] },
      { key: "any", values: ["500"] },
    ]);
  });

  it("a comma inside one term is OR, a space between terms is still AND", () => {
    expect(parseFilterTerms("tag:smoke,ios orders")).toEqual([
      { key: "tag", values: ["smoke", "ios"] },
      { key: "any", values: ["orders"] },
    ]);
  });

  it("drops the blanks a stray comma leaves behind", () => {
    // Each of these would otherwise contribute an empty needle, and an empty
    // needle matches every row — the failure mode is a filter that silently
    // stops filtering.
    expect(parseFilterTerms("tag:smoke,,")).toEqual([
      { key: "tag", values: ["smoke"] },
    ]);
    expect(parseFilterTerms(",")).toEqual([]);
    expect(parseFilterTerms("tag:,")).toEqual([{ key: "any", values: ["tag:"] }]);
  });
});

describe("matchesRuleFilter", () => {
  const terms = parseFilterTerms;

  it("matches everything when the query is empty", () => {
    expect(matchesRuleFilter(rule(), { name: "Checkout" }, terms(""))).toBe(true);
  });

  it("matches on the rule name, case-insensitively", () => {
    expect(matchesRuleFilter(rule(), { name: "Checkout" }, terms("ORDERS"))).toBe(true);
  });

  it("matches on the collection name — the group-name case", () => {
    expect(matchesRuleFilter(rule(), { name: "Checkout flow" }, terms("checkout"))).toBe(true);
  });

  it("matches on host, path and method of the request", () => {
    const r = rule({ name: "nothing relevant here" });
    expect(matchesRuleFilter(r, { name: "Grp" }, terms("api.example.com"))).toBe(true);
    expect(matchesRuleFilter(r, { name: "Grp" }, terms("/v2/orders"))).toBe(true);
    expect(matchesRuleFilter(r, { name: "Grp" }, terms("get"))).toBe(true);
  });

  it("matches on query params, name or value", () => {
    const r = rule({
      name: "x",
      match_path_glob: "/x",
      match_params: [{ name: "status", value: "shipped" }],
    });
    expect(matchesRuleFilter(r, { name: "Grp" }, terms("shipped"))).toBe(true);
    expect(matchesRuleFilter(r, { name: "Grp" }, terms("status=shipped"))).toBe(true);
  });

  it("ANDs several terms — all must match, order irrelevant", () => {
    const r = rule({ name: "Orders 500" });
    expect(matchesRuleFilter(r, { name: "Checkout" }, terms("orders 500"))).toBe(true);
    expect(matchesRuleFilter(r, { name: "Checkout" }, terms("500 orders"))).toBe(true);
    expect(matchesRuleFilter(r, { name: "Checkout" }, terms("orders 404"))).toBe(false);
  });

  it("ANDs across different fields — one term from the name, one from the host", () => {
    expect(matchesRuleFilter(rule(), { name: "Checkout" }, terms("orders api.example"))).toBe(
      true,
    );
  });

  it("does not match on the response side (status / body stay out)", () => {
    const r = rule({ name: "x", match_host_glob: "h", match_path_glob: "/p" });
    expect(matchesRuleFilter(r, { name: "Grp" }, terms("200"))).toBe(false);
  });

  it("survives null matchers instead of throwing", () => {
    const r = rule({
      name: "catch all",
      match_host_glob: null,
      match_method: null,
      match_path_glob: null,
    });
    expect(matchesRuleFilter(r, { name: "Grp" }, terms("catch"))).toBe(true);
    expect(matchesRuleFilter(r, { name: "Grp" }, terms("null"))).toBe(false);
  });
});

describe("ruleHaystack", () => {
  it("is lowercased so callers never have to normalise twice", () => {
    expect(ruleHaystack(rule(), { name: "Checkout" })).toBe(
      "orders empty list checkout get api.example.com /v2/orders",
    );
  });
});

describe("tags", () => {
  const terms = parseFilterTerms;

  it("a bare word finds a rule by its tag, like any other field", () => {
    const r = rule({ name: "x", match_host_glob: "h", match_path_glob: "/p", tags: ["smoke"] });
    expect(matchesRuleFilter(r, { name: "Grp" }, terms("smoke"))).toBe(true);
  });

  it("tag: narrows to the tags — the name no longer counts", () => {
    const named = rule({ name: "smoke test", tags: [] });
    const tagged = rule({ name: "nothing", match_host_glob: "h", match_path_glob: "/p", tags: ["smoke"] });
    expect(matchesRuleFilter(named, { name: "Grp" }, terms("tag:smoke"))).toBe(false);
    expect(matchesRuleFilter(tagged, { name: "Grp" }, terms("tag:smoke"))).toBe(true);
  });

  it("a rule inherits its collection's tags — labelling a group labels its rules", () => {
    const r = rule({ name: "x", match_host_glob: "h", match_path_glob: "/p" });
    expect(
      matchesRuleFilter(r, { name: "Grp", tags: ["scenario"] }, terms("tag:scenario")),
    ).toBe(true);
  });

  it("matches a tag by substring and ignores case, like every other term", () => {
    const r = rule({ name: "x", match_host_glob: "h", match_path_glob: "/p", tags: ["Регресс"] });
    expect(matchesRuleFilter(r, { name: "Grp" }, terms("TAG:регр"))).toBe(true);
  });

  it("ANDs a tag term with a plain one", () => {
    const r = rule({ name: "Orders empty", tags: ["smoke"] });
    expect(matchesRuleFilter(r, { name: "Grp" }, terms("tag:smoke orders"))).toBe(true);
    expect(matchesRuleFilter(r, { name: "Grp" }, terms("tag:smoke refunds"))).toBe(false);
  });

  it("finds either label with tag:a,b, and neither with a third", () => {
    const smoke = rule({ name: "x", match_host_glob: "h", match_path_glob: "/p", tags: ["smoke"] });
    const ios = rule({ name: "x", match_host_glob: "h", match_path_glob: "/p", tags: ["ios"] });
    const other = rule({ name: "x", match_host_glob: "h", match_path_glob: "/p", tags: ["perf"] });
    expect(matchesRuleFilter(smoke, { name: "Grp" }, terms("tag:smoke,ios"))).toBe(true);
    expect(matchesRuleFilter(ios, { name: "Grp" }, terms("tag:smoke,ios"))).toBe(true);
    expect(matchesRuleFilter(other, { name: "Grp" }, terms("tag:smoke,ios"))).toBe(false);
  });

  it("ORs inside a term but still ANDs across terms", () => {
    const r = rule({ name: "x", match_host_glob: "h", match_path_glob: "/p", tags: ["smoke", "v2"] });
    expect(matchesRuleFilter(r, { name: "Grp" }, terms("tag:smoke,ios tag:v2"))).toBe(true);
    expect(matchesRuleFilter(r, { name: "Grp" }, terms("tag:smoke,ios tag:v3"))).toBe(false);
  });

  it("half-typed `tag:` stays a plain term instead of matching everything tagged", () => {
    expect(parseFilterTerms("tag:")).toEqual([{ key: "any", values: ["tag:"] }]);
    const tagged = rule({ name: "x", match_host_glob: "h", match_path_glob: "/p", tags: ["smoke"] });
    expect(matchesRuleFilter(tagged, { name: "Grp" }, terms("tag:"))).toBe(false);
  });

  it("keeps a collection on screen when its own tag is typed", () => {
    expect(
      matchesCollection({ name: "Checkout", tags: ["scenario"] }, terms("tag:scenario")),
    ).toBe(true);
    expect(matchesCollection({ name: "Checkout" }, terms("tag:scenario"))).toBe(false);
  });
});

describe("matchesCollection", () => {
  it("keeps an empty group visible when its name is typed", () => {
    expect(matchesCollection({ name: "Checkout flow" }, parseFilterTerms("checkout"))).toBe(
      true,
    );
    expect(matchesCollection({ name: "Checkout flow" }, parseFilterTerms("orders"))).toBe(
      false,
    );
  });

  it("matches everything when the query is empty", () => {
    expect(matchesCollection({ name: "Anything" }, parseFilterTerms(""))).toBe(true);
  });

  // The captures context menu narrows its collection list through this, so
  // `tag:` and its comma-OR have to work there and not only in the Rules view.
  it("finds a collection by either of two tags", () => {
    const c = { name: "Checkout", tags: ["scenario"] };
    expect(matchesCollection(c, parseFilterTerms("tag:smoke,scenario"))).toBe(true);
    expect(matchesCollection(c, parseFilterTerms("tag:smoke,perf"))).toBe(false);
  });
});
