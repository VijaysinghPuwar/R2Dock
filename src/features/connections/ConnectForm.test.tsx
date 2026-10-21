import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ConnectionInput } from "../../ipc/bindings/ConnectionInput";
import type { ConnectionSummary } from "../../ipc/bindings/ConnectionSummary";
import { renderWithProviders } from "../../test/utils";
import { ConnectForm } from "./ConnectForm";

const commands = vi.hoisted(() => ({
  connectionTest: vi.fn(),
  connectionSave: vi.fn(),
}));

vi.mock("../../ipc/commands", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../ipc/commands")>();
  return { ...actual, connectionTest: commands.connectionTest, connectionSave: commands.connectionSave };
});

const SECRET = "s3cr3t-value-for-test";
const ACCOUNT = "a1b2c3d4e5f6a7b8c9d0e1f2a3b49f0e";

const summary: ConnectionSummary = {
  id: "conn-1",
  name: "Personal R2",
  accountId: ACCOUNT,
  accountIdShort: "a1b2...9f0e",
  accessKeyHint: "AKIA...WXYZ",
  jurisdiction: "default",
  secretStorage: "keychain",
  status: "online",
  bucketScoped: false,
  manualBuckets: [],
  favoriteBuckets: [],
  publicBaseUrls: {},
  hasApiToken: false,
  lastTestedAt: null,
  active: true,
};

function secretInput(): HTMLInputElement {
  return screen.getByLabelText("Secret Access Key") as HTMLInputElement;
}

async function fillNew(user: ReturnType<typeof userEvent.setup>) {
  await user.type(screen.getByLabelText("Name"), "Personal");
  await user.type(screen.getByLabelText("Account ID"), ACCOUNT);
  await user.type(screen.getByLabelText("Access Key ID"), "AKIATEST");
  await user.type(secretInput(), SECRET);
}

function renderNew(onSaved = vi.fn()) {
  renderWithProviders(
    <ConnectForm existing={null} credentialStoreAvailable onCancel={() => undefined} onSaved={onSaved} />,
  );
  return onSaved;
}

beforeEach(() => {
  commands.connectionTest.mockReset();
  commands.connectionSave.mockReset();
});

// These type long keys one key at a time, which is slow when every test
// file runs at once.
describe("ConnectForm", { timeout: 15_000 }, () => {
  it("keeps the secret after a failed test so it can be retried, and shows technical details on request", async () => {
    const user = userEvent.setup();
    commands.connectionTest.mockRejectedValue({
      code: "signatureMismatch",
      message: "The Secret Access Key does not match this Access Key ID. Paste the secret again or create a new token.",
      detail: "SignatureDoesNotMatch (request id 1234)",
      retryable: false,
    });
    renderNew();
    await fillNew(user);
    expect(secretInput().value).toBe(SECRET);

    await user.click(screen.getByRole("button", { name: "Test connection" }));

    expect(await screen.findByText(/does not match this Access Key ID/)).toBeInTheDocument();
    expect(commands.connectionTest).toHaveBeenCalledTimes(1);
    expect((commands.connectionTest.mock.calls[0]?.[0] as ConnectionInput).secret).toBe(SECRET);
    expect(secretInput().value).toBe(SECRET);

    // The error text never contains the secret.
    expect(document.body.textContent ?? "").not.toContain(SECRET);

    expect(screen.queryByText(/SignatureDoesNotMatch/)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Show technical details" }));
    expect(screen.getByText(/SignatureDoesNotMatch/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Hide technical details" }));
    expect(screen.queryByText(/SignatureDoesNotMatch/)).not.toBeInTheDocument();
  });

  it("keeps the secret after a passing test so Save can send it, then clears it after save", async () => {
    const user = userEvent.setup();
    commands.connectionTest.mockResolvedValue({ message: "Connected. Found 3 buckets.", bucketScoped: false, buckets: [], warning: null });
    commands.connectionSave.mockResolvedValue(summary);
    const onSaved = renderNew();
    await fillNew(user);

    await user.click(screen.getByRole("button", { name: "Test connection" }));
    expect(await screen.findByText("Connected. Found 3 buckets.")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Save connection" }));
    await waitFor(() => expect(onSaved).toHaveBeenCalledWith(summary));
    // Save never calls the test command (Rust tests before saving).
    expect(commands.connectionTest).toHaveBeenCalledTimes(1);
    expect((commands.connectionSave.mock.calls[0]?.[0] as ConnectionInput).secret).toBe(SECRET);
    expect(secretInput().value).toBe("");
  });

  it("keeps the secret when saving fails so the user can fix and retry", async () => {
    const user = userEvent.setup();
    commands.connectionTest.mockResolvedValue({ message: "Connected. Found 1 bucket.", bucketScoped: false, buckets: [], warning: null });
    commands.connectionSave.mockRejectedValue({ code: "credentialStoreUnavailable", message: "Could not save to the keychain.", detail: null, retryable: false });
    const onSaved = renderNew();
    await fillNew(user);

    await user.click(screen.getByRole("button", { name: "Save connection" }));
    expect(await screen.findByText("Could not save to the keychain.")).toBeInTheDocument();
    expect(onSaved).not.toHaveBeenCalled();
    expect(commands.connectionTest).not.toHaveBeenCalled();
    expect(secretInput().value).toBe(SECRET);
  });

  it("shows validation errors and calls nothing when fields are empty", async () => {
    const user = userEvent.setup();
    renderNew();
    await user.click(screen.getByRole("button", { name: "Save connection" }));
    expect(screen.getByText("Enter a name.")).toBeInTheDocument();
    expect(screen.getByText("Enter the Secret Access Key.")).toBeInTheDocument();
    expect(commands.connectionTest).not.toHaveBeenCalled();
    expect(commands.connectionSave).not.toHaveBeenCalled();
  });

  it("sends secret null in edit mode when the secret is not replaced", async () => {
    const user = userEvent.setup();
    commands.connectionTest.mockResolvedValue({ message: "Connected. Found 3 buckets.", bucketScoped: false, buckets: [], warning: null });
    commands.connectionSave.mockResolvedValue(summary);
    const onSaved = vi.fn();
    renderWithProviders(
      <ConnectForm existing={summary} credentialStoreAvailable onCancel={() => undefined} onSaved={onSaved} />,
    );

    expect(screen.getByText("Stored in keychain")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Replace secret" })).toBeInTheDocument();
    expect(screen.queryByLabelText("Secret Access Key")).not.toBeInTheDocument();

    const name = screen.getByLabelText("Name");
    await user.clear(name);
    await user.type(name, "Work R2");
    await user.click(screen.getByRole("button", { name: "Save connection" }));

    await waitFor(() => expect(onSaved).toHaveBeenCalled());
    const input = commands.connectionSave.mock.calls[0]?.[0] as ConnectionInput;
    expect(input.id).toBe("conn-1");
    expect(input.name).toBe("Work R2");
    expect(input.secret).toBeNull();
  });

  it("requires a new secret after choosing Replace secret in edit mode", async () => {
    const user = userEvent.setup();
    renderWithProviders(
      <ConnectForm existing={summary} credentialStoreAvailable onCancel={() => undefined} onSaved={vi.fn()} />,
    );
    await user.click(screen.getByRole("button", { name: "Replace secret" }));
    expect(secretInput()).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Save connection" }));
    expect(screen.getByText("Paste the new Secret Access Key.")).toBeInTheDocument();
    expect(commands.connectionSave).not.toHaveBeenCalled();
  });

  it("sends the optional API token with save and clears it afterwards", async () => {
    const user = userEvent.setup();
    const TOKEN = "fake-api-token-for-test";
    commands.connectionSave.mockResolvedValue(summary);
    const onSaved = renderNew();
    await fillNew(user);
    const tokenInput = screen.getByLabelText("API token (optional)") as HTMLInputElement;
    expect(screen.getByText(/Shown as Token value when you create the R2 token/)).toBeInTheDocument();
    await user.type(tokenInput, TOKEN);

    await user.click(screen.getByRole("button", { name: "Save connection" }));
    await waitFor(() => expect(onSaved).toHaveBeenCalledWith(summary));
    const input = commands.connectionSave.mock.calls[0]?.[0] as ConnectionInput;
    expect(input.apiToken).toBe(TOKEN);
    expect(input.removeApiToken).toBe(false);
    expect(tokenInput.value).toBe("");
    expect(document.body.textContent ?? "").not.toContain(TOKEN);
  });

  it("sends a null API token when none is typed", async () => {
    const user = userEvent.setup();
    commands.connectionSave.mockResolvedValue(summary);
    renderNew();
    await fillNew(user);
    await user.click(screen.getByRole("button", { name: "Save connection" }));
    await waitFor(() => expect(commands.connectionSave).toHaveBeenCalled());
    expect((commands.connectionSave.mock.calls[0]?.[0] as ConnectionInput).apiToken).toBeNull();
  });

  it("keeps or removes a stored API token in edit mode without showing it", async () => {
    const user = userEvent.setup();
    commands.connectionSave.mockResolvedValue(summary);
    const existing = { ...summary, hasApiToken: true };
    renderWithProviders(
      <ConnectForm existing={existing} credentialStoreAvailable onCancel={() => undefined} onSaved={vi.fn()} />,
    );
    expect(screen.queryByLabelText("API token (optional)")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Replace token" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Remove token" }));
    expect(screen.getByText("Removed when you save")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Save connection" }));
    await waitFor(() => expect(commands.connectionSave).toHaveBeenCalled());
    const input = commands.connectionSave.mock.calls[0]?.[0] as ConnectionInput;
    expect(input.apiToken).toBeNull();
    expect(input.removeApiToken).toBe(true);
  });

  it("explains session-only storage when the keychain is unavailable", () => {
    renderWithProviders(
      <ConnectForm existing={null} credentialStoreAvailable={false} onCancel={() => undefined} onSaved={vi.fn()} />,
    );
    expect(screen.getByText(/Kept for this session only/)).toBeInTheDocument();
    expect(screen.getByText(/system keychain is not available/)).toBeInTheDocument();
  });
});
