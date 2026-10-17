import type { GroupInfo } from "../../ipc/bindings/GroupInfo";
import type { JobSnapshot } from "../../ipc/bindings/JobSnapshot";
import { joinDot } from "../../lib/format";
import { tabOf, type TransferTab } from "./stats";

export type DirectionFilter = "all" | "upload" | "download";

export type ListItem =
  | { type: "job"; job: JobSnapshot; child: boolean }
  | { type: "group"; group: GroupInfo; jobs: JobSnapshot[]; all: JobSnapshot[]; expanded: boolean };

export function filterDirection(jobs: JobSnapshot[], filter: DirectionFilter): JobSnapshot[] {
  if (filter === "all") return jobs;
  return jobs.filter((j) => j.direction === filter);
}

/** Text for a group row: "Uploading folder X · 87 of 195 files". */
export function groupText(group: GroupInfo, all: JobSnapshot[]): string {
  const done = all.filter((j) => j.state === "completed").length;
  const upload = (all[0]?.direction ?? "upload") === "upload";
  const finished = all.every((j) => j.state === "completed" || j.state === "skipped" || j.state === "cancelled" || j.state === "failed");
  const verb = finished ? (upload ? "Uploaded" : "Downloaded") : upload ? "Uploading" : "Downloading";
  return joinDot([`${verb} folder ${group.label}`, `${done.toLocaleString("en-US")} of ${all.length.toLocaleString("en-US")} files`]);
}

/**
 * Build display items for one tab. Jobs that share a group are shown under a
 * group row, placed where the first matching child appears.
 */
export function buildItems(jobs: JobSnapshot[], tab: TransferTab, expanded: ReadonlySet<string>): ListItem[] {
  const byGroup = new Map<string, JobSnapshot[]>();
  for (const j of jobs) {
    if (!j.group) continue;
    const list = byGroup.get(j.group.id);
    if (list) list.push(j);
    else byGroup.set(j.group.id, [j]);
  }
  const items: ListItem[] = [];
  const placed = new Set<string>();
  for (const j of jobs) {
    if (tabOf(j.state) !== tab) continue;
    if (!j.group) {
      items.push({ type: "job", job: j, child: false });
      continue;
    }
    const gid = j.group.id;
    if (placed.has(gid)) continue;
    placed.add(gid);
    const all = byGroup.get(gid) ?? [j];
    const inTab = all.filter((c) => tabOf(c.state) === tab);
    const isOpen = expanded.has(gid);
    items.push({ type: "group", group: j.group, jobs: inTab, all, expanded: isOpen });
    if (isOpen) for (const c of inTab) items.push({ type: "job", job: c, child: true });
  }
  return items;
}
