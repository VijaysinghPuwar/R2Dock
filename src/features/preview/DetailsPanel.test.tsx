import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ConnectionSummary } from "../../ipc/bindings/ConnectionSummary";
import type { ObjectDetails } from "../../ipc/bindings/ObjectDetails";
import type { PublicLink } from "../../ipc/bindings/PublicLink";
import { makeConnection } from "../../test/connections";
import { renderWithProviders } from "../../test/utils";
import type { FileRow } from "../browser/rows";
import { DetailsPanel } from "./DetailsPanel";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
  convertFileSrc: () => "r2dock-media://x",
  Channel: class {
    onmessage: ((m: unknown) => void) | null = null;
  },
}));

const KEY = "Audiobook G/cover art.zip";
const ENCODED = "Audiobook%20G/cover%20art.zip";

const row: FileRow = {
  kind: "file",
  id: KEY,
  entry: {
    key: KEY,
    name: "cover art.zip",
    displayName: "cover art.zip",
    size: 2048,
    lastModified: "2026-10-02T10:00:00Z",
    etag: null,
    storageClass: null,
    unusual: false,
  },
};

const details: ObjectDetails = {
  key: KEY,
  name: "cover art.zip",
  displayName: "cover art.zip",
  size: 2048,
  contentType: "application/zip",
  lastModified: "2026-10-02T10:00:00Z",
  etag: null,
  etagIsMd5: false,
  storageClass: "Standard",
  cacheControl: null,
  contentEncoding: null,
  metadata: {},
};

function mockCommands(link: PublicLink | null) {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "object_head") return Promise.resolve({ details });
    if (cmd === "object_public_url" && link) return Promise.resolve(link);
    if (cmd === "open_external" || cmd === "clipboard_write") return Promise.resolve(null);
    return Promise.reject(new Error(`unexpected ${cmd}`));
  });
}

function render(connection: ConnectionSummary) {
  renderWithProviders(
    <DetailsPanel connection={connection} bucket="media-library" prefix="Audiobook G/" rows={[row]} overlay={false} onClose={() => undefined} />,
  );
}

const calls = (cmd: string) => invokeMock.mock.calls.filter(([c]) => c === cmd);

beforeEach(() => {
  invokeMock.mockReset();
});

describe("DetailsPanel public link", () => {
  it("shows a detected custom domain with copy and open actions", async () => {
    const user = userEvent.setup();
    const url = `https://media.example.com/${ENCODED}`;
    mockCommands({ url, source: "customDomain", status: "found" });
    render(makeConnection({ hasApiToken: true }));

    expect(await screen.findByText(url)).toBeInTheDocument();
    expect(screen.getByText("Custom domain")).toBeInTheDocument();
    expect(calls("object_public_url")[0]?.[1]).toEqual({ connectionId: "conn-1", bucket: "media-library", key: KEY });

    await user.click(screen.getByRole("button", { name: "Open in browser" }));
    await waitFor(() => expect(calls("open_external")).toHaveLength(1));
    expect(calls("open_external")[0]?.[1]).toEqual({ url });
    expect(screen.getByRole("button", { name: "Copy Custom domain link" })).toBeInTheDocument();
  });

  it("labels a manual public URL", async () => {
    mockCommands({ url: `https://cdn.example.com/${ENCODED}`, source: "publicUrl", status: "found" });
    render(makeConnection({ publicBaseUrls: { "media-library": "https://cdn.example.com" } }));
    expect(await screen.findByText(`https://cdn.example.com/${ENCODED}`)).toBeInTheDocument();
    expect(screen.getByText("Public URL")).toBeInTheDocument();
  });

  it("labels the r2.dev domain", async () => {
    const url = `https://pub-00000000000000000000000000000000.r2.dev/${ENCODED}`;
    mockCommands({ url, source: "r2Dev", status: "found" });
    render(makeConnection({ hasApiToken: true }));
    expect(await screen.findByText(url)).toBeInTheDocument();
    expect(screen.getByText("r2.dev")).toBeInTheDocument();
  });

  it("does not ask Rust without a public URL or API token, and mentions the token", async () => {
    mockCommands(null);
    render(makeConnection());
    expect(await screen.findByText(/Add an API token to the connection/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Bucket settings" })).toBeInTheDocument();
    expect(calls("object_public_url")).toHaveLength(0);
  });

  it("shows a calm hint when the token may not read the bucket's domains", async () => {
    mockCommands({ url: null, source: null, status: "tokenDenied" });
    render(makeConnection({ hasApiToken: true }));
    expect(await screen.findByText(/cannot read this bucket's domains/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Open in browser" })).not.toBeInTheDocument();
  });

  it("shows a hint when the bucket has no public domain", async () => {
    mockCommands({ url: null, source: null, status: "noDomain" });
    render(makeConnection({ hasApiToken: true }));
    expect(await screen.findByText(/no public custom domain or r2.dev access/)).toBeInTheDocument();
  });
});
