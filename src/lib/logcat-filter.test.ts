import { describe, expect, it } from "vitest";
import { APP_NO_MATCH_PID, compileLogcatFilter, resolveAppPids } from "@/lib/logcat-filter";

// Regression coverage for the "app: filter shows the whole firehose" bug:
// `app:itrack` was resolving to an empty include list, which the SQL layer
// reads as "no app constraint" — so every unrelated row (system_server,
// com.google.*, hwservicemanager, …) leaked through instead of an empty view.
describe("resolveAppPids", () => {
  const names = new Map<number, Set<string>>([
    [111, new Set(["ru.frosteye.itrack"])],
    [222, new Set(["com.google.android.gms"])],
    [333, new Set(["system_server"])],
  ]);

  it("no app: terms → empty include/exclude (imposes no constraint)", () => {
    const { include, exclude } = resolveAppPids([], names);
    expect(include.size).toBe(0);
    expect(exclude.size).toBe(0);
  });

  it("positive app: matches a live pid by substring", () => {
    const { appPackages } = compileLogcatFilter("app:itrack");
    const { include, exclude } = resolveAppPids(appPackages, names);
    expect([...include]).toEqual([111]);
    expect(exclude.size).toBe(0);
  });

  it("positive app: matching no live pid → sentinel, NOT an empty include", () => {
    // The bug in the video: with no matching PID the include set was empty,
    // and an empty include silently means "show everything". It must instead
    // carry the impossible sentinel so the query matches zero rows.
    const { appPackages } = compileLogcatFilter("app:itrack");
    const { include } = resolveAppPids(appPackages, new Map());
    expect([...include]).toEqual([APP_NO_MATCH_PID]);
  });

  it("negated-only app: → exclude populated, include stays empty (no sentinel)", () => {
    // Pure-negative `app:!x` means "hide x, show the rest" — an empty include
    // is correct here (it is genuinely unconstrained), so no sentinel.
    const { appPackages } = compileLogcatFilter("app:!gms");
    const { include, exclude } = resolveAppPids(appPackages, names);
    expect(include.size).toBe(0);
    expect([...exclude]).toEqual([222]);
  });

  it("mixed pos+neg where the positive matches nothing → sentinel + exclude", () => {
    const { appPackages } = compileLogcatFilter("app:itrack,!gms");
    const { include, exclude } = resolveAppPids(
      appPackages,
      new Map([[222, new Set(["com.google.android.gms"])]]),
    );
    expect(include.has(APP_NO_MATCH_PID)).toBe(true);
    expect([...exclude]).toEqual([222]);
  });
});
