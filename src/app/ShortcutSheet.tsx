import { FolderOpen } from "lucide-react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { Kbd } from "../components/Controls";
import { toast } from "../components/toastStore";
import { errorMessage, openActivityFolder } from "../ipc/commands";
import { modKeyLabel } from "../lib/platform";
import { usePlatform } from "./queries";
import { useUiStore } from "./uiStore";

type Shortcut = [string[], string];

export function shortcutList(mod: string, mac: boolean): Shortcut[] {
  return [
    [[mod, "U"], "Upload files"],
    [[mod, "Shift", "U"], "Upload folder"],
    [[mod, "Shift", "N"], "New folder"],
    [[mod, "F"], "Focus search"],
    [[mod, "R"], "Refresh listing"],
    [["F5"], "Refresh listing"],
    [[mod, "A"], "Select all loaded items"],
    [mac ? [mod, "Backspace"] : ["Delete"], "Delete selected"],
    [[mod, "["], "Back"],
    [[mod, "]"], "Forward"],
    [["Backspace"], "Parent folder"],
    [["Alt", "Up"], "Parent folder"],
    [[mod, "1"], "List view"],
    [[mod, "2"], "Grid view"],
    [[mod, "I"], "Details panel"],
    [[mod, "Shift", "C"], "Copy object key"],
    [[mod, "J"], "Transfer dock"],
    [[mod, ","], "Settings"],
    [["Space"], "Quick preview"],
    [["Enter"], "Open"],
    [["Esc"], "Clear selection or close"],
    [["?"], "This list"],
  ];
}

export function ShortcutSheet() {
  const open = useUiStore((s) => s.shortcutsOpen);
  const setOpen = useUiStore((s) => s.setShortcutsOpen);
  const platform = usePlatform();
  if (!open) return null;
  const items = shortcutList(modKeyLabel(platform), platform === "macos");
  return (
    <Dialog open onOpenChange={setOpen} title="Keyboard shortcuts" width="review">
      <dl className="shortcut-grid">
        {items.map(([keys, label], i) => (
          <div key={`${label}-${i}`} className="shortcut-row">
            <dt>{label}</dt>
            <dd className="shortcut-keys">
              {keys.map((k) => (
                <Kbd key={k}>{k}</Kbd>
              ))}
            </dd>
          </div>
        ))}
      </dl>
      <section className="shortcut-help" aria-labelledby="shortcut-help-title">
        <h3 id="shortcut-help-title" className="shortcut-help-title">
          Help
        </h3>
        <Button
          size="sm"
          icon={<FolderOpen size={14} />}
          onClick={() => openActivityFolder().catch((e: unknown) => toast.danger(errorMessage(e)))}
        >
          Open activity log folder
        </Button>
      </section>
    </Dialog>
  );
}
