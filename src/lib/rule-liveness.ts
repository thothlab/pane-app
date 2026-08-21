/**
 * Is a rule live for one device?
 *
 * The engine decides this in SQL (`Storage::list_active_rules`); this mirrors
 * it so the rules list can show the same answer without a round trip. Keep the
 * two in step: a list that disagrees with the proxy is worse than one that
 * shows the raw flag, because it looks authoritative.
 */
import type { RuleDto } from "../ipc/types";

/** Scope id for this Mac's own traffic. Mirrors `pane_storage::SCOPE_HOST`. */
export const SCOPE_HOST = "__host__";

/**
 * `deviceId === null` means "no device picked" — the global plane, where the
 * question is simply whether the rule is on at all.
 */
export function isLiveOn(rule: RuleDto, deviceId: string | null): boolean {
  if (!rule.enabled) return false;
  if (deviceId === null) return true;
  if ((rule.enabled_scope ?? "all") === "all") return true;
  return (rule.devices ?? []).includes(deviceId);
}

/** Does this rule name specific devices, rather than running everywhere? */
export function isScoped(rule: RuleDto): boolean {
  return (rule.enabled_scope ?? "all") === "set";
}

export type TriState = "on" | "off" | "mixed";

/** Header checkbox state for a group of rules, seen from one device. */
export function groupState(rules: RuleDto[], deviceId: string | null): TriState {
  if (rules.length === 0) return "off";
  const on = rules.filter((r) => isLiveOn(r, deviceId)).length;
  if (on === 0) return "off";
  if (on === rules.length) return "on";
  return "mixed";
}
