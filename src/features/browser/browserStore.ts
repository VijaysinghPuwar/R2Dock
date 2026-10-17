import { create } from "zustand";
import { DEFAULT_SORT, type SelectableRow, type SortState } from "./rows";

export type SearchScope = "folder" | "bucket";

interface BrowserState {
  /** Selected ids (keys for files, prefixes for folders). */
  selected: Set<string>;
  /** Anchor for Shift-click ranges. */
  anchor: string | null;
  /** Keyboard focus row id. */
  focusId: string | null;
  /** The rows currently shown, in display order (for shortcuts and preview). */
  visibleRows: SelectableRow[];

  search: string;
  scope: SearchScope;
  sort: SortState;

  setSelected: (ids: Iterable<string>, anchor?: string | null) => void;
  toggle: (id: string) => void;
  clearSelection: () => void;
  setFocus: (id: string | null) => void;
  setVisibleRows: (rows: SelectableRow[]) => void;
  setSearch: (search: string) => void;
  setScope: (scope: SearchScope) => void;
  setSort: (sort: SortState) => void;
  /** Reset per-folder state when the location changes. */
  resetForLocation: () => void;
}

export const useBrowserStore = create<BrowserState>((set) => ({
  selected: new Set(),
  anchor: null,
  focusId: null,
  visibleRows: [],
  search: "",
  scope: "folder",
  sort: DEFAULT_SORT,

  setSelected: (ids, anchor) => set((s) => ({ selected: new Set(ids), anchor: anchor === undefined ? s.anchor : anchor })),
  toggle: (id) =>
    set((s) => {
      const next = new Set(s.selected);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return { selected: next, anchor: id };
    }),
  clearSelection: () => set({ selected: new Set(), anchor: null }),
  setFocus: (focusId) => set({ focusId }),
  setVisibleRows: (visibleRows) => set({ visibleRows }),
  setSearch: (search) => set({ search }),
  setScope: (scope) => set({ scope }),
  setSort: (sort) => set({ sort }),
  resetForLocation: () => set({ selected: new Set(), anchor: null, focusId: null, search: "", scope: "folder" }),
}));

/** Selected rows in display order. */
export function selectedRows(state: { selected: Set<string>; visibleRows: SelectableRow[] }): SelectableRow[] {
  if (state.selected.size === 0) return [];
  return state.visibleRows.filter((r) => state.selected.has(r.id));
}
