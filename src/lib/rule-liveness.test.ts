import { describe, expect, it } from "vitest";

import { groupState, isLiveOn, isScoped, SCOPE_HOST } from "./rule-liveness";
import type { RuleDto } from "../ipc/types";

function rule(over: Partial<RuleDto> = {}): RuleDto {
  return {
    id: "r1",
    name: "orders-500",
    enabled: true,
    enabled_scope: "all",
    devices: [],
    priority: 0,
    collection_id: null,
    mode: "stub",
    patches: [],
    match_host_glob: "api.example.com",
    match_method: null,
    match_path_glob: null,
    match_params: [],
    match_req_body: null,
    match_conditions: [],
    res_status: 200,
    res_headers: [],
    res_body_id: null,
    res_body_mime: null,
    res_body_size: 0,
    res_delay_ms: 0,
    created_at: "0",
    updated_at: "0",
    ...over,
  } as RuleDto;
}

describe("isLiveOn", () => {
  it("an all-devices rule is live everywhere", () => {
    const r = rule();
    expect(isLiveOn(r, null)).toBe(true);
    expect(isLiveOn(r, "pixel")).toBe(true);
    expect(isLiveOn(r, SCOPE_HOST)).toBe(true);
  });

  it("the flag still dominates the scope", () => {
    const r = rule({ enabled: false, enabled_scope: "set", devices: ["pixel"] });
    expect(isLiveOn(r, "pixel")).toBe(false);
  });

  it("a scoped rule is live only on the devices it names", () => {
    const r = rule({ enabled_scope: "set", devices: ["pixel"] });
    expect(isLiveOn(r, "pixel")).toBe(true);
    expect(isLiveOn(r, "emulator-5554")).toBe(false);
  });

  // A ticked rule that names nobody must read as off, not as on-everywhere —
  // getting this backwards would show a mock as live on all four phones.
  it("a rule scoped to nobody is live nowhere", () => {
    const r = rule({ enabled_scope: "set", devices: [] });
    expect(isLiveOn(r, "pixel")).toBe(false);
  });

  it("with no device picked, the question is just the flag", () => {
    expect(isLiveOn(rule({ enabled_scope: "set", devices: ["pixel"] }), null)).toBe(true);
    expect(isLiveOn(rule({ enabled: false }), null)).toBe(false);
  });
});

describe("isScoped", () => {
  it("distinguishes a pinned rule from a global one", () => {
    expect(isScoped(rule())).toBe(false);
    expect(isScoped(rule({ enabled_scope: "set", devices: ["pixel"] }))).toBe(true);
  });
});

describe("groupState", () => {
  const onPixel = rule({ id: "a", enabled_scope: "set", devices: ["pixel"] });
  const onEmu = rule({ id: "b", enabled_scope: "set", devices: ["emu"] });

  it("reads per device, so one group can differ between phones", () => {
    expect(groupState([onPixel, onEmu], "pixel")).toBe("mixed");
    expect(groupState([onPixel], "pixel")).toBe("on");
    expect(groupState([onPixel], "emu")).toBe("off");
  });

  it("an empty group is off, not mixed", () => {
    expect(groupState([], "pixel")).toBe("off");
  });
});
