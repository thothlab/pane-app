/**
 * Saved-filters store, scoped by kind ("captures" | "logcat").
 *
 * Each kind has its own signal cached at module scope so all consumers of
 * the same kind (e.g. CapturesView + Layout, or LogcatView's star + dropdown)
 * react to the same source of truth without polling. Different webviews
 * (main vs logcat windows) each have their own module instance and thus
 * their own independent signals.
 */

import { createSignal, type Accessor } from "solid-js";

import { api } from "@/ipc/client";
import type { FilterDto, FilterKind } from "@/ipc/types";

interface KindStore {
  filters: Accessor<FilterDto[]>;
  refresh: () => Promise<void>;
  save: (args: SaveArgs) => Promise<FilterDto>;
  remove: (id: string) => Promise<void>;
}

interface SaveArgs {
  /** When set, updates the existing row instead of creating a new one. */
  id?: string;
  name: string;
  query: string;
  color: string;
  pinned: boolean;
}

const stores = new Map<FilterKind, KindStore>();

function makeStore(kind: FilterKind): KindStore {
  const [filters, setFilters] = createSignal<FilterDto[]>([]);

  const refresh = async () => {
    try {
      setFilters(await api.filters.list(kind));
    } catch (e) {
      console.warn(`filters list (${kind}) failed`, e);
    }
  };

  const save = async (args: SaveArgs): Promise<FilterDto> => {
    const saved = await api.filters.save({ ...args, kind });
    await refresh();
    return saved;
  };

  const remove = async (id: string): Promise<void> => {
    await api.filters.delete(id);
    await refresh();
  };

  return { filters, refresh, save, remove };
}

export function savedFiltersFor(kind: FilterKind): KindStore {
  let s = stores.get(kind);
  if (!s) {
    s = makeStore(kind);
    stores.set(kind, s);
  }
  return s;
}

// Convenience accessors for the captures-scoped store — preserves the
// previous import surface so CapturesView / Layout don't change shape.
const captures = savedFiltersFor("captures");
export const filters = captures.filters;
export const refreshFilters = captures.refresh;
export const saveFilter = captures.save;
export const deleteFilter = captures.remove;
