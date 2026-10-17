import { create } from "zustand";
import type { ViewMode } from "../ipc/bindings/ViewMode";

export type ConnectDialogState =
  | { mode: "new"; showHelp: boolean }
  | { mode: "edit"; id: string; replaceSecret: boolean; showHelp: boolean };

interface UiState {
  sidebarCollapsed: boolean;
  setSidebarCollapsed: (collapsed: boolean) => void;
  narrow: boolean;
  setNarrow: (narrow: boolean) => void;

  detailsOpen: boolean | null;
  setDetailsOpen: (open: boolean) => void;
  viewMode: ViewMode | null;
  setViewMode: (mode: ViewMode) => void;

  dockExpanded: boolean;
  setDockExpanded: (expanded: boolean) => void;

  connectDialog: ConnectDialogState | null;
  openConnect: (state?: ConnectDialogState) => void;
  closeConnect: () => void;

  shortcutsOpen: boolean;
  setShortcutsOpen: (open: boolean) => void;

  /** Bumped to ask the toolbar search field to take focus. */
  searchFocusTick: number;
  focusSearch: () => void;

  newFolderOpen: boolean;
  setNewFolderOpen: (open: boolean) => void;
  goToPathOpen: boolean;
  setGoToPathOpen: (open: boolean) => void;
  bucketSettings: { connectionId: string; bucket: string } | null;
  setBucketSettings: (target: { connectionId: string; bucket: string } | null) => void;
}

/** Small UI-only state. Preferences live in the settings query. */
export const useUiStore = create<UiState>((set) => ({
  sidebarCollapsed: typeof window !== "undefined" ? window.innerWidth < 1200 : false,
  setSidebarCollapsed: (sidebarCollapsed) => set({ sidebarCollapsed }),
  narrow: typeof window !== "undefined" ? window.innerWidth < 1200 : false,
  setNarrow: (narrow) => set({ narrow }),

  detailsOpen: null,
  setDetailsOpen: (detailsOpen) => set({ detailsOpen }),
  viewMode: null,
  setViewMode: (viewMode) => set({ viewMode }),

  dockExpanded: false,
  setDockExpanded: (dockExpanded) => set({ dockExpanded }),

  connectDialog: null,
  openConnect: (state) => set({ connectDialog: state ?? { mode: "new", showHelp: false } }),
  closeConnect: () => set({ connectDialog: null }),

  shortcutsOpen: false,
  setShortcutsOpen: (shortcutsOpen) => set({ shortcutsOpen }),

  searchFocusTick: 0,
  focusSearch: () => set((s) => ({ searchFocusTick: s.searchFocusTick + 1 })),

  newFolderOpen: false,
  setNewFolderOpen: (newFolderOpen) => set({ newFolderOpen }),
  goToPathOpen: false,
  setGoToPathOpen: (goToPathOpen) => set({ goToPathOpen }),
  bucketSettings: null,
  setBucketSettings: (bucketSettings) => set({ bucketSettings }),
}));
