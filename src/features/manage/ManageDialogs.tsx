import { DeleteFlow } from "./DeleteFlow";
import { GoToPathDialog, NewFolderDialog } from "./FolderDialogs";
import { MoveToFlow } from "./MoveToFlow";
import { RenameFlow } from "./RenameFlow";

/** Hosts delete, rename, move, new folder, and go-to-path dialogs. */
export function ManageDialogs() {
  return (
    <>
      <DeleteFlow />
      <RenameFlow />
      <MoveToFlow />
      <NewFolderDialog />
      <GoToPathDialog />
    </>
  );
}
