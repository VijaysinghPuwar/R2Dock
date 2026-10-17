import { create } from "zustand";
import { errorMessage, pickUploadFiles, pickUploadFolder, uploadPrepare, uploadStart } from "../../ipc/commands";
import type { ConflictPolicy } from "../../ipc/bindings/ConflictPolicy";
import type { Selection } from "../../ipc/bindings/Selection";
import type { UploadPlan } from "../../ipc/bindings/UploadPlan";
import { toast } from "../../components/toastStore";
import { folderLabel } from "../../lib/keys";
import { plural } from "../../lib/format";

export interface UploadTarget {
  connectionId: string;
  connectionName: string;
  bucket: string;
  prefix: string;
}

interface PendingReview {
  plan: UploadPlan;
  target: UploadTarget;
  selection: Selection;
}

interface UploadState {
  review: PendingReview | null;
  preparing: boolean;
  setReview: (review: PendingReview | null) => void;
  setPreparing: (preparing: boolean) => void;
}

/** Upload plans waiting for review. Plans hold no secrets and no local paths. */
export const useUploadStore = create<UploadState>((set) => ({
  review: null,
  preparing: false,
  setReview: (review) => set({ review }),
  setPreparing: (preparing) => set({ preparing }),
}));

/** Start a reviewed or simple plan. */
export async function startPlan(
  plan: UploadPlan,
  target: UploadTarget,
  policy: ConflictPolicy,
  compress = true,
): Promise<boolean> {
  try {
    const jobs = await uploadStart(plan.planId, policy, compress);
    toast.neutral(`Uploading ${plural(jobs, "file")} to ${folderLabel(target.bucket, target.prefix)}.`);
    return true;
  } catch (e) {
    toast.danger(errorMessage(e));
    return false;
  }
}

/** Prepare a selection; show the review dialog when needed, otherwise start right away. */
export async function uploadSelection(selection: Selection, target: UploadTarget): Promise<void> {
  const store = useUploadStore.getState();
  store.setPreparing(true);
  try {
    const plan = await uploadPrepare(selection.id, target.connectionId, target.bucket, target.prefix);
    if (plan.needsReview) {
      useUploadStore.getState().setReview({ plan, target, selection });
      return;
    }
    if (plan.files === 0) {
      toast.warning("Nothing to upload.");
      return;
    }
    await startPlan(plan, target, plan.defaultPolicy);
  } catch (e) {
    toast.danger(errorMessage(e));
  } finally {
    useUploadStore.getState().setPreparing(false);
  }
}

/** Upload files or a folder chosen with the system picker. */
export async function pickAndUpload(kind: "files" | "folder", target: UploadTarget): Promise<void> {
  try {
    const selection = kind === "files" ? await pickUploadFiles() : await pickUploadFolder();
    if (!selection) return; // cancelled
    await uploadSelection(selection, target);
  } catch (e) {
    toast.danger(errorMessage(e));
  }
}
