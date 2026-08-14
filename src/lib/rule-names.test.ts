import { describe, expect, it } from "vitest";
import { RULE_NAME_MAX, uniqueRuleName } from "@/lib/rule-names";

describe("uniqueRuleName", () => {
  it("leaves a first-seen name alone", () => {
    const used = new Set<string>();
    expect(uniqueRuleName("GET api.example.com/orders", used)).toBe(
      "GET api.example.com/orders",
    );
  });

  it("suffixes repeats of the same generated name", () => {
    // The multi-select case: three captures of one polling endpoint inside
    // the same minute all generate the identical stamped name.
    const used = new Set<string>();
    const name = "GET api.example.com/ping · 2026-08-12 21:45";
    expect(uniqueRuleName(name, used)).toBe(name);
    expect(uniqueRuleName(name, used)).toBe(`${name} (2)`);
    expect(uniqueRuleName(name, used)).toBe(`${name} (3)`);
  });

  it("keeps different endpoints untouched", () => {
    const used = new Set<string>();
    uniqueRuleName("GET a/1", used);
    expect(uniqueRuleName("GET a/2", used)).toBe("GET a/2");
  });

  it("does not blow the name budget when the base is already at the cap", () => {
    const used = new Set<string>();
    const long = "G".repeat(RULE_NAME_MAX);
    expect(uniqueRuleName(long, used)).toBe(long);
    const second = uniqueRuleName(long, used);
    expect(second.length).toBe(RULE_NAME_MAX);
    expect(second.endsWith(" (2)")).toBe(true);
  });

  it("keeps suffixing past a collision it created itself", () => {
    // A trimmed "… (2)" can itself collide with a name the user already
    // has; the loop must keep walking rather than return a duplicate.
    const used = new Set<string>();
    const long = "G".repeat(RULE_NAME_MAX);
    used.add(long);
    used.add(`${long.slice(0, RULE_NAME_MAX - 4)} (2)`);
    const out = uniqueRuleName(long, used);
    expect(out).toBe(`${long.slice(0, RULE_NAME_MAX - 4)} (3)`);
  });
});
