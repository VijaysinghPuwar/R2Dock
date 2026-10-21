import { beforeEach, describe, expect, it } from "vitest";
import { useNavStore } from "./navStore";
import { useUiStore } from "./uiStore";

describe("folder dialogs", () => {
  beforeEach(() => {
    useNavStore.getState().reset("buckets");
    useNavStore.getState().openBucket("media", "a/");
  });

  it("close when the shown folder changes", () => {
    useUiStore.getState().setNewFolderOpen(true);
    useUiStore.getState().setGoToPathOpen(true);
    useNavStore.getState().goBack();
    expect(useUiStore.getState().newFolderOpen).toBe(false);
    expect(useUiStore.getState().goToPathOpen).toBe(false);
  });

  it("stay open when the folder stays the same", () => {
    useUiStore.getState().setNewFolderOpen(true);
    useNavStore.getState().openBucket("media", "a/");
    expect(useUiStore.getState().newFolderOpen).toBe(true);
    useUiStore.getState().setNewFolderOpen(false);
  });
});
