import { create } from "zustand";

export type ToastTone = "neutral" | "success" | "warning" | "danger";

export interface ToastAction {
  label: string;
  run: () => void;
}

export interface ToastItem {
  id: number;
  tone: ToastTone;
  message: string;
  action?: ToastAction;
}

interface ToastState {
  toasts: ToastItem[];
  push: (toast: Omit<ToastItem, "id">) => number;
  dismiss: (id: number) => void;
  clear: () => void;
}

let nextId = 1;

/** Toast queue. At most 3 are visible; the rest wait. */
export const useToastStore = create<ToastState>((set) => ({
  toasts: [],
  push: (t) => {
    const id = nextId++;
    set((s) => ({ toasts: [...s.toasts, { ...t, id }] }));
    return id;
  },
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
  clear: () => set({ toasts: [] }),
}));

export const MAX_VISIBLE_TOASTS = 3;
export const TOAST_TIMEOUT_MS = 5000;

function push(tone: ToastTone, message: string, action?: ToastAction): number {
  return useToastStore.getState().push({ tone, message, action });
}

export const toast = {
  neutral: (message: string, action?: ToastAction) => push("neutral", message, action),
  success: (message: string, action?: ToastAction) => push("success", message, action),
  warning: (message: string, action?: ToastAction) => push("warning", message, action),
  danger: (message: string, action?: ToastAction) => push("danger", message, action),
  dismiss: (id: number) => useToastStore.getState().dismiss(id),
};
