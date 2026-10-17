import { create } from "zustand";
import { parentPrefix } from "../lib/keys";

export type View = "welcome" | "buckets" | "browser" | "transfers" | "settings" | "recent" | "favorites";
export type SettingsSection =
  | "connections"
  | "appearance"
  | "transfers"
  | "compression"
  | "security"
  | "notifications"
  | "advanced"
  | "about";

export interface Location {
  view: View;
  bucket: string | null;
  prefix: string;
}

interface NavState {
  loc: Location;
  back: Location[];
  forward: Location[];
  settingsSection: SettingsSection;
  /** Go somewhere new; pushes the current location onto the back stack. */
  navigate: (to: Partial<Location> & { view: View }) => void;
  openBucket: (bucket: string, prefix?: string) => void;
  openPrefix: (prefix: string) => void;
  goUp: () => void;
  goBack: () => void;
  goForward: () => void;
  openSettings: (section?: SettingsSection) => void;
  setSettingsSection: (section: SettingsSection) => void;
  /** Start over (for example after switching connection). */
  reset: (view: View) => void;
}

const HISTORY_LIMIT = 100;

function same(a: Location, b: Location): boolean {
  return a.view === b.view && a.bucket === b.bucket && a.prefix === b.prefix;
}

export const useNavStore = create<NavState>((set, get) => ({
  loc: { view: "buckets", bucket: null, prefix: "" },
  back: [],
  forward: [],
  settingsSection: "connections",
  navigate: (to) => {
    const cur = get().loc;
    const next: Location = {
      view: to.view,
      bucket: to.bucket !== undefined ? to.bucket : to.view === "browser" ? cur.bucket : null,
      prefix: to.prefix ?? "",
    };
    if (same(cur, next)) return;
    set((s) => ({ loc: next, back: [...s.back, cur].slice(-HISTORY_LIMIT), forward: [] }));
  },
  openBucket: (bucket, prefix = "") => get().navigate({ view: "browser", bucket, prefix }),
  openPrefix: (prefix) => {
    const { bucket } = get().loc;
    if (bucket) get().navigate({ view: "browser", bucket, prefix });
  },
  goUp: () => {
    const { loc } = get();
    if (loc.view !== "browser" || !loc.bucket) return;
    if (loc.prefix === "") {
      get().navigate({ view: "buckets" });
      return;
    }
    get().navigate({ view: "browser", bucket: loc.bucket, prefix: parentPrefix(loc.prefix) });
  },
  goBack: () => {
    const { back, loc } = get();
    const prev = back[back.length - 1];
    if (!prev) return;
    set((s) => ({ loc: prev, back: s.back.slice(0, -1), forward: [loc, ...s.forward].slice(0, HISTORY_LIMIT) }));
  },
  goForward: () => {
    const { forward, loc } = get();
    const next = forward[0];
    if (!next) return;
    set((s) => ({ loc: next, forward: s.forward.slice(1), back: [...s.back, loc].slice(-HISTORY_LIMIT) }));
  },
  openSettings: (section) => {
    if (section) set({ settingsSection: section });
    get().navigate({ view: "settings" });
  },
  setSettingsSection: (section) => set({ settingsSection: section }),
  reset: (view) => set({ loc: { view, bucket: null, prefix: "" }, back: [], forward: [] }),
}));
