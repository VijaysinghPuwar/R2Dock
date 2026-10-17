import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ObjectEntry } from "../../ipc/bindings/ObjectEntry";
import { visibleText } from "../../lib/keys";
import { renderWithProviders } from "../../test/utils";
import { FileTable, type FileTableProps } from "./FileTable";
import { DEFAULT_SORT, fileRow, folderRow, type Row } from "./rows";

const RLO = String.fromCharCode(0x202e);

function entry(key: string, displayName?: string, extra: Partial<ObjectEntry> = {}): ObjectEntry {
  const name = key.split("/").pop() ?? key;
  return {
    key,
    name,
    displayName: displayName ?? name,
    size: 412_000,
    lastModified: "2026-10-02T09:00:00Z",
    etag: null,
    storageClass: "STANDARD",
    unusual: false,
    ...extra,
  };
}

function renderTable(rows: Row[], props: Partial<FileTableProps> = {}) {
  const handlers = {
    onRowClick: vi.fn(),
    onRowDoubleClick: vi.fn(),
    onToggle: vi.fn(),
    onKeyDown: vi.fn(),
    onContextMenu: vi.fn(),
    onLoadMore: vi.fn(),
    onSort: vi.fn(),
  };
  renderWithProviders(
    <FileTable
      rows={rows}
      selected={new Set()}
      focusId={null}
      rowHeight={32}
      hasMore={false}
      loadingMore={false}
      label="Contents of covers"
      sort={DEFAULT_SORT}
      {...handlers}
      {...props}
    />,
  );
  return handlers;
}

describe("FileTable", () => {
  it("renders a bidi-override name as visible text, never as HTML", () => {
    const raw = `invoice${RLO}fdp.exe`;
    const shown = visibleText(raw);
    const html = '<img src=x onerror="alert(1)">.png';
    renderTable([
      fileRow(entry(`docs/${raw}`, shown, { unusual: true })),
      fileRow(entry(`docs/${html}`)),
    ]);

    // The placeholder is visible and the raw override character is not in the DOM text.
    expect(screen.getAllByTitle(shown).length).toBeGreaterThan(0);
    expect(document.body.textContent).toContain("U+202E");
    expect(document.body.textContent).not.toContain(RLO);

    // Markup in a name is plain text.
    expect(document.querySelector("img")).toBeNull();
    expect(screen.getAllByTitle(html).length).toBeGreaterThan(0);

    // Unusual keys get a warning badge.
    expect(screen.getByRole("img", { name: "Unusual key name" })).toBeInTheDocument();
  });

  it("is a grid with an unknown row count while more pages exist", () => {
    renderTable([folderRow({ prefix: "covers/", name: "covers", displayName: "covers", unusual: false })], { hasMore: true });
    const grid = screen.getByRole("grid", { name: "Contents of covers" });
    expect(grid).toHaveAttribute("aria-rowcount", "-1");
    expect(screen.getByText("Showing 1 of more objects. Loading more...")).toBeInTheDocument();
  });

  it("shows -- for folder sizes and a full row count when complete", () => {
    renderTable([
      folderRow({ prefix: "ch 1-195/", name: "ch 1-195", displayName: "ch 1-195", unusual: false }),
      fileRow(entry("cover.webp")),
    ]);
    expect(screen.getByRole("grid")).toHaveAttribute("aria-rowcount", "3");
    const folder = screen.getAllByRole("row").find((r) => within(r).queryByText("ch 1-195"));
    expect(folder).toBeDefined();
    if (folder) expect(within(folder).getAllByRole("gridcell")[3]).toHaveTextContent("--");
    expect(screen.getByText("412 KB")).toBeInTheDocument();
    expect(screen.getByText("WebP image")).toBeInTheDocument();
  });

  it("reports clicks, double clicks, and header sorting", async () => {
    const user = userEvent.setup();
    const handlers = renderTable([fileRow(entry("cover.webp"))]);
    const row = screen.getAllByRole("row")[1];
    if (!row) throw new Error("row missing");
    await user.click(row);
    expect(handlers.onRowClick).toHaveBeenCalledTimes(1);
    await user.dblClick(row);
    expect(handlers.onRowDoubleClick).toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Size" }));
    expect(handlers.onSort).toHaveBeenCalledWith("size");
  });
});
