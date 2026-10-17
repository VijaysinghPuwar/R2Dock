import { create } from "zustand";
import type { ObjectEntry } from "../../ipc/bindings/ObjectEntry";

interface PreviewState {
  open: boolean;
  connectionId: string;
  bucket: string;
  /** Files of the current folder, in display order. */
  items: ObjectEntry[];
  index: number;
  show: (connectionId: string, bucket: string, items: ObjectEntry[], index: number) => void;
  setIndex: (index: number) => void;
  close: () => void;
}

export const usePreviewStore = create<PreviewState>((set) => ({
  open: false,
  connectionId: "",
  bucket: "",
  items: [],
  index: 0,
  show: (connectionId, bucket, items, index) => set({ open: true, connectionId, bucket, items, index }),
  setIndex: (index) => set({ index }),
  close: () => set({ open: false, items: [] }),
}));
