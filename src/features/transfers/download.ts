import { downloadStart, errorMessage } from "../../ipc/commands";
import { toast } from "../../components/toastStore";
import { plural } from "../../lib/format";

/** Start downloading keys and folder prefixes. Rust picks or asks for the destination. */
export async function startDownload(connectionId: string, bucket: string, keys: string[], prefixes: string[]): Promise<void> {
  if (keys.length === 0 && prefixes.length === 0) return;
  try {
    const started = await downloadStart(connectionId, bucket, keys, prefixes);
    if (!started) return; // the user cancelled the folder picker
    toast.neutral(`Downloading ${plural(started.jobs, "file")} to ${started.destination}`);
  } catch (e) {
    toast.danger(errorMessage(e));
  }
}
