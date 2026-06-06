/**
 * App-wide i18n using `@solid-primitives/i18n`.
 *
 * Usage in components:
 *   import { t } from "@/i18n";
 *   <button>{t()("nav.start_proxy")}</button>
 *
 * `t` is a Solid memo: calling `t()` inside JSX gives the current
 * translator, and Solid re-renders when the locale changes. Keys are
 * dot-notation paths into the flattened dictionary.
 *
 * Adding a new locale:
 *   1. Create `src/i18n/<lang>.ts` exporting the same shape as `en.ts`.
 *   2. Register it in the `dictionaries` map below.
 *   3. Add an entry to `LOCALES`.
 *
 * Default locale is English on first launch (per product call); the
 * user picks their language in Settings, and we persist the choice in
 * localStorage so it survives restarts.
 */

import { createMemo, createSignal } from "solid-js";
import * as i18n from "@solid-primitives/i18n";

import en from "./en";
import ru from "./ru";

export type Locale = "en" | "ru";

/** Human-readable labels for the language picker. */
export const LOCALES: { code: Locale; label: string }[] = [
  { code: "en", label: "English" },
  { code: "ru", label: "Русский" },
];

const DEFAULT_LOCALE: Locale = "en";
const STORAGE_KEY = "pane:locale";

// Pre-flatten so each lookup is a single Map hit instead of walking
// nested objects. Done once at module load, dictionaries are static.
const dictionaries = {
  en: i18n.flatten(en),
  ru: i18n.flatten(ru),
};

function loadLocale(): Locale {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === "en" || raw === "ru") return raw;
  } catch {
    /* private mode / SSR */
  }
  return DEFAULT_LOCALE;
}

const [locale, setLocaleSignal] = createSignal<Locale>(loadLocale());

/** Active locale signal. Read with `locale()`. */
export { locale };

/** Persist + apply a new locale. Triggers a re-render of every
 *  component that reads `t()`. */
export function setLocale(next: Locale): void {
  try {
    localStorage.setItem(STORAGE_KEY, next);
  } catch {
    /* swallow — locale still changes in-memory */
  }
  setLocaleSignal(next);
}

/** Reactive translator. Call as `t()(key, params?)` inside JSX.
 *  `resolveTemplate` is the critical 2nd argument — without it, strings
 *  like "Update to v{version}" come out literally as "Update to v{version}".
 *  @solid-primitives/i18n's default resolver is identity (no substitution),
 *  so you opt into template interpolation explicitly. */
export const t = createMemo(() =>
  i18n.translator(() => dictionaries[locale()], i18n.resolveTemplate),
);

/** Imperative one-off (for non-reactive call sites: throw new Error,
 *  prompt(), confirm(), etc). Doesn't track the locale signal. */
export function tr(key: string, params?: Record<string, string | number>): string {
  const fn = i18n.translator(() => dictionaries[locale()], i18n.resolveTemplate);
  // The translator's type is templated on the dict path-set, which a
  // dynamic string can't satisfy. Cast through `any` here — call sites
  // using the reactive `t()` get full type safety; this escape hatch
  // is for non-JSX places (alert/confirm) where reactivity is moot.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  return ((fn as any)(key, params) as string) ?? key;
}
