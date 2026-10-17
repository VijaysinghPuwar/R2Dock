import {
  ArrowUpDown,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Copy,
  Download,
  Ellipsis,
  FolderInput,
  FolderPlus,
  Globe,
  LayoutGrid,
  List,
  TextCursorInput,
  Ruler,
  RotateCw,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useMediaQuery } from "../../lib/useMediaQuery";
import { Button, IconButton } from "../../components/Button";
import { SegmentedControl } from "../../components/Controls";
import { SearchField } from "../../components/Fields";
import { Menu, MenuCheckboxItem, MenuItem, MenuLabel, MenuRadioGroup, MenuSeparator } from "../../components/Menu";
import type { Density } from "../../ipc/bindings/Density";
import type { ViewMode } from "../../ipc/bindings/ViewMode";
import { modKeyLabel } from "../../lib/platform";
import { folderLabel, splitPrefix } from "../../lib/keys";
import { formatCount } from "../../lib/format";
import { usePlatform } from "../../app/queries";
import { useNavStore } from "../../app/navStore";
import { useUiStore } from "../../app/uiStore";
import type { SearchScope } from "./browserStore";
import type { SortKey, SortState } from "./rows";

function Breadcrumb({ connectionName, bucket, prefix }: { connectionName: string; bucket: string; prefix: string }) {
  const navigate = useNavStore((s) => s.navigate);
  const openBucket = useNavStore((s) => s.openBucket);
  const crumbs = splitPrefix(prefix);
  const atRoot = crumbs.length === 0;
  // When the toolbar is short on space, drop the connection name and fold
  // more folders into the "..." menu instead of squeezing every segment.
  const navRef = useRef<HTMLElement>(null);
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const el = navRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => setWidth(entries[0]?.contentRect.width ?? 0));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  const narrow = width > 0 && width < 520;
  const keep = narrow ? 1 : 2;
  const collapse = crumbs.length > (narrow ? keep : keep + 1);
  const hidden = collapse ? crumbs.slice(0, crumbs.length - keep) : [];
  const shown = collapse ? crumbs.slice(-keep) : crumbs;

  const sep = (
    <span className="crumb-sep" aria-hidden="true">
      /
    </span>
  );
  const items: ReactNode[] = [];
  if (!narrow) {
    items.push(
      <li key="conn">
        <button type="button" className="crumb" onClick={() => navigate({ view: "buckets" })}>
          {connectionName}
        </button>
      </li>,
    );
  }
  items.push(
    <li key="bucket">
      {!narrow && sep}
      {atRoot ? (
        <span className="crumb is-current" aria-current="page">
          {bucket}
        </span>
      ) : (
        <button type="button" className="crumb" onClick={() => openBucket(bucket, "")}>
          {bucket}
        </button>
      )}
    </li>,
  );
  if (hidden.length > 0) {
    items.push(
      <li key="more">
        {sep}
        <Menu
          trigger={
            <button type="button" className="crumb" aria-label="Show hidden folders">
              ...
            </button>
          }
        >
          {hidden.map((c) => (
            <MenuItem key={c.prefix} onSelect={() => openBucket(bucket, c.prefix)}>
              <span className={c.empty ? "muted" : undefined}>{c.label}</span>
            </MenuItem>
          ))}
        </Menu>
      </li>,
    );
  }
  shown.forEach((c, i) => {
    const last = i === shown.length - 1;
    items.push(
      <li key={c.prefix}>
        {sep}
        {last ? (
          <span className={c.empty ? "crumb is-current is-empty" : "crumb is-current"} aria-current="page" title={c.label}>
            {c.label}
          </span>
        ) : (
          <button type="button" className={c.empty ? "crumb is-empty" : "crumb"} onClick={() => openBucket(bucket, c.prefix)} title={c.label}>
            {c.label}
          </button>
        )}
      </li>,
    );
  });
  return (
    <nav ref={navRef} aria-label="Breadcrumb" className="breadcrumb">
      <ol>{items}</ol>
    </nav>
  );
}

export interface ToolbarProps {
  connectionName: string;
  bucket: string;
  prefix: string;
  selectedCount: number;
  onDownloadSelected: () => void;
  onMoveSelected: () => void;
  onCopyKeys: () => void;
  onDeleteSelected: () => void;
  onClearSelection: () => void;
  search: string;
  onSearch: (value: string) => void;
  scope: SearchScope;
  onScope: (scope: SearchScope) => void;
  onUpload: (kind: "files" | "folder") => void;
  onNewFolder: () => void;
  onRefresh: () => void;
  sort: SortState;
  onSort: (sort: SortState) => void;
  viewMode: ViewMode;
  onViewMode: (mode: ViewMode) => void;
  density: Density;
  onDensity: (density: Density) => void;
  detailsOpen: boolean;
  onDetailsOpen: (open: boolean) => void;
  onCopyPath: () => void;
  onBucketSettings: () => void;
  onCalculateSize: () => void;
  uploading: boolean;
}

const SORT_OPTIONS: Array<{ value: SortKey; label: string }> = [
  { value: "name", label: "Name" },
  { value: "size", label: "Size" },
  { value: "modified", label: "Modified" },
  { value: "type", label: "Type" },
];

export function Toolbar(props: ToolbarProps) {
  const platform = usePlatform();
  const mod = modKeyLabel(platform);
  const back = useNavStore((s) => s.back.length > 0);
  const forward = useNavStore((s) => s.forward.length > 0);
  const goBack = useNavStore((s) => s.goBack);
  const goForward = useNavStore((s) => s.goForward);
  const focusTick = useUiStore((s) => s.searchFocusTick);
  const searchRef = useRef<HTMLInputElement>(null);
  const firstTick = useRef(focusTick);

  useEffect(() => {
    if (focusTick !== firstTick.current) {
      searchRef.current?.focus();
      searchRef.current?.select();
    }
  }, [focusTick]);

  const where = folderLabel(props.bucket, props.prefix);
  const selection = props.selectedCount > 0;
  // On narrower windows the selection actions become icon buttons so none are cut off.
  const compactSelection = useMediaQuery("(max-width: 1360px)");
  const copyLabel = props.selectedCount === 1 ? "Copy key" : "Copy keys";

  return (
    <div className="toolbar" role="toolbar" aria-label="Folder actions">
      <div className="toolbar-left">
        {selection ? (
          <div className="selection-bar" role="group" aria-label="Selection">
            <IconButton label="Clear selection" shortcut="Esc" size="sm" icon={<X size={14} />} onClick={props.onClearSelection} />
            <span className="selection-count num">{formatCount(props.selectedCount)} selected</span>
            {compactSelection ? (
              <>
                <IconButton label="Download" size="sm" icon={<Download size={14} />} onClick={props.onDownloadSelected} />
                <IconButton label="Move to..." size="sm" icon={<FolderInput size={14} />} onClick={props.onMoveSelected} />
                <IconButton label={copyLabel} size="sm" icon={<Copy size={14} />} onClick={props.onCopyKeys} />
                <IconButton label="Delete" size="sm" className="danger-text" icon={<Trash2 size={14} />} onClick={props.onDeleteSelected} />
              </>
            ) : (
              <>
                <Button size="sm" variant="ghost" icon={<Download size={14} />} onClick={props.onDownloadSelected}>
                  Download
                </Button>
                <Button size="sm" variant="ghost" icon={<FolderInput size={14} />} onClick={props.onMoveSelected}>
                  Move to...
                </Button>
                <Button size="sm" variant="ghost" icon={<Copy size={14} />} onClick={props.onCopyKeys}>
                  {copyLabel}
                </Button>
                <Button size="sm" variant="ghost" className="danger-text" icon={<Trash2 size={14} />} onClick={props.onDeleteSelected}>
                  Delete
                </Button>
              </>
            )}
          </div>
        ) : (
          <>
            <IconButton label="Back" shortcut={`${mod}+[`} icon={<ChevronLeft size={16} />} onClick={goBack} disabled={!back} />
            <IconButton label="Forward" shortcut={`${mod}+]`} icon={<ChevronRight size={16} />} onClick={goForward} disabled={!forward} />
            <Breadcrumb connectionName={props.connectionName} bucket={props.bucket} prefix={props.prefix} />
          </>
        )}
      </div>

      <div className="toolbar-right">
        <SearchField
          inputRef={searchRef}
          value={props.search}
          onChange={props.onSearch}
          placeholder={props.scope === "bucket" ? `Keys starting with...` : `Search in ${where}`}
          label={props.scope === "bucket" ? `Search keys in ${props.bucket}` : `Search in ${where}`}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              if (props.search) props.onSearch("");
              else (e.target as HTMLInputElement).blur();
            }
          }}
          scope={{
            label: props.scope === "folder" ? "This folder" : "Whole bucket",
            title:
              props.scope === "folder"
                ? "Filters names already loaded in this folder. Click to search the whole bucket by key prefix."
                : "Finds keys that start with the text, across the whole bucket. Click to search this folder only.",
            onToggle: () => props.onScope(props.scope === "folder" ? "bucket" : "folder"),
          }}
        />
        <div className="split-btn">
          <Button variant="primary" icon={<Upload size={16} />} onClick={() => props.onUpload("files")} loading={props.uploading}>
            Upload
          </Button>
          <Menu
            align="end"
            trigger={
              <Button variant="primary" aria-label="More upload options" icon={<ChevronDown size={14} />} />
            }
          >
            <MenuItem icon={<Upload size={16} />} shortcut={`${mod}+U`} onSelect={() => props.onUpload("files")}>
              Upload files...
            </MenuItem>
            <MenuItem icon={<FolderPlus size={16} />} shortcut={`${mod}+Shift+U`} onSelect={() => props.onUpload("folder")}>
              Upload folder...
            </MenuItem>
          </Menu>
        </div>
        <Button icon={<FolderPlus size={16} />} onClick={props.onNewFolder} className="hide-narrow-label">
          <span className="label-text">New folder</span>
        </Button>
        <IconButton label="Refresh" shortcut={`${mod}+R`} icon={<RotateCw size={16} />} onClick={props.onRefresh} />
        <Menu
          align="end"
          trigger={<IconButton label="Sort" icon={<ArrowUpDown size={16} />} />}
        >
          <MenuLabel>Sort by</MenuLabel>
          <MenuRadioGroup
            value={props.sort.key}
            onValueChange={(key) => props.onSort({ ...props.sort, key })}
            options={SORT_OPTIONS}
          />
          <MenuSeparator />
          <MenuRadioGroup
            value={props.sort.dir}
            onValueChange={(dir) => props.onSort({ ...props.sort, dir })}
            options={[
              { value: "asc", label: "Ascending" },
              { value: "desc", label: "Descending" },
            ]}
          />
          <MenuSeparator />
          <MenuCheckboxItem
            checked={props.sort.foldersFirst}
            onCheckedChange={(foldersFirst) => props.onSort({ ...props.sort, foldersFirst })}
          >
            Folders first
          </MenuCheckboxItem>
        </Menu>
        <SegmentedControl
          label="View"
          value={props.viewMode}
          onChange={props.onViewMode}
          options={[
            { value: "list", label: "List view", icon: <List size={14} />, iconOnly: true },
            { value: "grid", label: "Grid view", icon: <LayoutGrid size={14} />, iconOnly: true },
          ]}
        />
        <Menu align="end" trigger={<IconButton label="More" icon={<Ellipsis size={16} />} />}>
          <MenuLabel>Density</MenuLabel>
          <MenuRadioGroup
            value={props.density}
            onValueChange={props.onDensity}
            options={[
              { value: "comfortable", label: "Comfortable" },
              { value: "compact", label: "Compact" },
            ]}
          />
          <MenuSeparator />
          <MenuCheckboxItem checked={props.detailsOpen} onCheckedChange={props.onDetailsOpen} shortcut={`${mod}+I`}>
            Show details panel
          </MenuCheckboxItem>
          <MenuSeparator />
          <MenuItem icon={<Copy size={16} />} onSelect={props.onCopyPath}>
            Copy current path
          </MenuItem>
          <MenuItem icon={<Globe size={16} />} onSelect={props.onBucketSettings}>
            Bucket settings...
          </MenuItem>
          <MenuItem icon={<Ruler size={16} />} onSelect={props.onCalculateSize}>
            Calculate folder size
          </MenuItem>
          <MenuSeparator />
          <MenuItem icon={<TextCursorInput size={16} />} onSelect={() => useUiStore.getState().setGoToPathOpen(true)}>
            Go to path...
          </MenuItem>
        </Menu>
      </div>
    </div>
  );
}
