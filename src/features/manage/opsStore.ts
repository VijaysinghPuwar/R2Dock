import { create } from "zustand";

export interface OpContext {
  connectionId: string;
  bucket: string;
}

export interface DeleteRequest extends OpContext {
  keys: string[];
  prefixes: string[];
  /** Display name for a single item (already safe for display). */
  label: string;
}

export type RenameTarget =
  | { kind: "file"; key: string; name: string; displayName: string }
  | { kind: "folder"; prefix: string; name: string; displayName: string };

export interface RenameRequest extends OpContext {
  target: RenameTarget;
}

export interface MoveToRequest extends OpContext {
  keys: string[];
  prefixes: string[];
  /** Folder the items currently live in. */
  fromPrefix: string;
}

interface OpsState {
  del: DeleteRequest | null;
  rename: RenameRequest | null;
  moveTo: MoveToRequest | null;
  requestDelete: (req: DeleteRequest) => void;
  requestRename: (req: RenameRequest) => void;
  requestMoveTo: (req: MoveToRequest) => void;
  closeDelete: () => void;
  closeRename: () => void;
  closeMoveTo: () => void;
}

/** Pending delete, rename, and move requests. ManageDialogs renders them. */
export const useOpsStore = create<OpsState>((set) => ({
  del: null,
  rename: null,
  moveTo: null,
  requestDelete: (del) => set({ del }),
  requestRename: (rename) => set({ rename }),
  requestMoveTo: (moveTo) => set({ moveTo }),
  closeDelete: () => set({ del: null }),
  closeRename: () => set({ rename: null }),
  closeMoveTo: () => set({ moveTo: null }),
}));
