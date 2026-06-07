/**
 * Saved-filters store: shared list of FilterDto, kept in sync with the
 * backend `filter` table via `api.filters`.
 *
 * Owned at module scope so CapturesView (saves) and Layout (lists, applies,
 * deletes) react to the same signal without polling.
 */

import { createSignal } from "solid-js";

import { api } from "@/ipc/client";
import type { FilterDto } from "@/ipc/types";

const [filters, setFilters] = createSignal<FilterDto[]>([]);
export { filters };

export async function refreshFilters(): Promise<void> {
  try {
    setFilters(await api.filters.list());
  } catch (e) {
    console.warn("filters list failed", e);
  }
}

export async function saveFilter(args: {
  /** When set, updates the existing row instead of creating a new one. */
  id?: string;
  name: string;
  query: string;
  color: string;
  pinned: boolean;
}): Promise<FilterDto> {
  const saved = await api.filters.save(args);
  await refreshFilters();
  return saved;
}

export async function deleteFilter(id: string): Promise<void> {
  await api.filters.delete(id);
  await refreshFilters();
}
