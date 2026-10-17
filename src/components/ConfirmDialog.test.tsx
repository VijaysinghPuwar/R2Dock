import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderWithProviders } from "../test/utils";
import { TypedConfirmDialog } from "./ConfirmDialog";

describe("TypedConfirmDialog", () => {
  it("keeps Delete disabled until the phrase matches exactly", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    renderWithProviders(
      <TypedConfirmDialog
        open
        onOpenChange={() => undefined}
        title='Delete folder "ch 1-195" and 195 objects inside?'
        phrase="ch 1-195"
        confirmLabel="Delete"
        onConfirm={onConfirm}
      />,
    );

    const input = screen.getByLabelText(/Type ch 1-195 to confirm/);
    const del = screen.getByRole("button", { name: "Delete" });
    expect(del).toBeDisabled();

    await user.type(input, "ch 1-19");
    expect(del).toBeDisabled();

    await user.type(input, "5 ");
    expect(del).toBeDisabled();

    await user.clear(input);
    await user.type(input, "CH 1-195");
    expect(del).toBeDisabled();

    await user.clear(input);
    await user.type(input, "ch 1-195");
    expect(del).toBeEnabled();

    await user.click(del);
    expect(onConfirm).toHaveBeenCalledWith("ch 1-195");
  });

  it("does not submit with Enter before the phrase matches", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    renderWithProviders(
      <TypedConfirmDialog open onOpenChange={() => undefined} title="Delete 60 objects?" phrase="delete 60" confirmLabel="Delete" onConfirm={onConfirm} />,
    );
    await user.type(screen.getByLabelText(/to confirm/), "delete 6{Enter}");
    expect(onConfirm).not.toHaveBeenCalled();
  });
});
