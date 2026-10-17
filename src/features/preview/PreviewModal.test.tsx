import { act, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ObjectEntry } from "../../ipc/bindings/ObjectEntry";
import { renderWithProviders } from "../../test/utils";
import { PreviewModal } from "./PreviewModal";
import { usePreviewStore } from "./previewStore";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
  convertFileSrc: () => "r2dock-media://x",
  Channel: class {
    onmessage: ((m: unknown) => void) | null = null;
  },
}));

function entry(name: string, size = 100): ObjectEntry {
  return {
    key: `docs/${name}`,
    name,
    displayName: name,
    size,
    lastModified: "2026-10-02T10:00:00Z",
    etag: null,
    storageClass: null,
    unusual: false,
  };
}

function open(items: ObjectEntry[], index = 0) {
  act(() => {
    usePreviewStore.getState().show("conn-1", "media", items, index);
  });
}

beforeEach(() => {
  invokeMock.mockReset();
  usePreviewStore.getState().close();
});

describe("PreviewModal", () => {
  it("renders text previews as literal text, never as HTML", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "preview_text") {
        return Promise.resolve({ text: "<script>alert(1)</script>", truncated: true, binary: false });
      }
      return Promise.reject(new Error(`unexpected ${cmd}`));
    });
    open([entry("page.html")]);
    renderWithProviders(<PreviewModal />);

    const pre = await screen.findByText("<script>alert(1)</script>");
    expect(pre.tagName).toBe("PRE");
    expect(document.querySelector("script")).toBeNull();
    expect(screen.getByText("Showing first 1 MB")).toBeInTheDocument();
  });

  it("says when a file does not look like text", async () => {
    invokeMock.mockResolvedValue({ text: "", truncated: false, binary: true });
    open([entry("notes.txt")]);
    renderWithProviders(<PreviewModal />);
    expect(await screen.findByText("This file does not look like text")).toBeInTheDocument();
  });

  it("asks before loading a large image", async () => {
    invokeMock.mockImplementation((cmd: string) => (cmd === "media_token" ? Promise.resolve("tok") : Promise.reject(new Error(cmd))));
    open([entry("huge.png", 48_000_000)]);
    renderWithProviders(<PreviewModal />);

    expect(screen.getByText("Large image (48.0 MB). Preview anyway?")).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith("media_token", expect.anything());

    await userEvent.click(screen.getByRole("button", { name: "Preview anyway" }));
    const img = await screen.findByRole("img", { name: "huge.png" });
    expect(img).toHaveAttribute("src", "r2dock-media://x");
  });

  it("shows a fallback for unsupported types", () => {
    open([entry("archive.zip")]);
    renderWithProviders(<PreviewModal />);
    expect(screen.getByText("No preview for this file type")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Download" }).length).toBeGreaterThan(0);
  });

  it("moves between files with the arrow keys", async () => {
    open([entry("a.zip"), entry("b.zip")]);
    renderWithProviders(<PreviewModal />);
    expect(screen.getByRole("heading", { name: "a.zip" })).toBeInTheDocument();
    await userEvent.keyboard("{ArrowRight}");
    expect(screen.getByRole("heading", { name: "b.zip" })).toBeInTheDocument();
    await userEvent.keyboard("{ArrowLeft}");
    expect(screen.getByRole("heading", { name: "a.zip" })).toBeInTheDocument();
  });
});
