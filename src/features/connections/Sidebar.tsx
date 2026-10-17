import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ArrowDownUp,
  Check,
  ChevronDown,
  ChevronRight,
  Clock,
  Database,
  Info,
  PanelLeftClose,
  PanelLeftOpen,
  Plus,
  Settings,
  SlidersHorizontal,
  Star,
} from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { Button, IconButton } from "../../components/Button";
import { StatusDot } from "../../components/Controls";
import { LogoMark } from "../../components/Logo";
import { Menu, MenuItem, MenuSeparator } from "../../components/Menu";
import { Tooltip } from "../../components/Tooltip";
import { toast } from "../../components/toastStore";
import { errorMessage, prefixSize } from "../../ipc/commands";
import type { PrefixSize } from "../../ipc/bindings/PrefixSize";
import { formatAgo, formatBytes, formatCount, joinDot, shortId } from "../../lib/format";
import { folderLabel } from "../../lib/keys";
import { useNavStore, type View } from "../../app/navStore";
import { qk, useActiveConnection, useBuckets, useConnections } from "../../app/queries";
import { useUiStore } from "../../app/uiStore";
import { isMoving } from "../transfers/stats";
import { useTransferStore } from "../transfers/transferStore";
import { statusLabel, statusTone } from "./status";
import { useSwitchConnection } from "./useSwitchConnection";
import "./sidebar.css";

interface NavItemProps {
  icon: ReactNode;
  label: string;
  active?: boolean;
  onClick: () => void;
  collapsed: boolean;
  badge?: number;
  disabled?: boolean;
  shortcut?: string;
}

function NavItem({ icon, label, active, onClick, collapsed, badge, disabled }: NavItemProps) {
  const button = (
    <button
      type="button"
      className={active ? "nav-item is-active" : "nav-item"}
      onClick={onClick}
      aria-current={active ? "page" : undefined}
      aria-label={collapsed ? (badge ? `${label}, ${badge} active` : label) : undefined}
      disabled={disabled}
    >
      <span className="nav-icon" aria-hidden="true">
        {icon}
      </span>
      {!collapsed && <span className="nav-label truncate">{label}</span>}
      {!!badge && (
        <span className={collapsed ? "nav-badge is-dot num" : "nav-badge num"} aria-hidden={collapsed || undefined}>
          {badge}
        </span>
      )}
    </button>
  );
  if (!collapsed) return button;
  return (
    <Tooltip content={label} side="right">
      {button}
    </Tooltip>
  );
}

function ConnectionSwitcher({ collapsed }: { collapsed: boolean }) {
  const { data: connections = [] } = useConnections();
  const active = useActiveConnection();
  const openConnect = useUiStore((s) => s.openConnect);
  const openSettings = useNavStore((s) => s.openSettings);
  const switchTo = useSwitchConnection();
  if (!active) return null;

  const trigger = collapsed ? (
    <button type="button" className="conn-monogram" aria-label={`Connection: ${active.name}. Switch connection`}>
      {active.name.trim().charAt(0).toUpperCase() || "R"}
      <span className="conn-monogram-dot">
        <StatusDot tone={statusTone(active.status)} />
      </span>
    </button>
  ) : (
    <button type="button" className="conn-switcher" aria-label={`Connection: ${active.name}. Switch connection`}>
      <StatusDot tone={statusTone(active.status)} />
      <span className="conn-name truncate">{active.name}</span>
      <ChevronDown size={14} aria-hidden="true" className="conn-chevron" />
    </button>
  );

  return (
    <Menu trigger={trigger} align="start">
      {connections.map((c) => (
        <MenuItem
          key={c.id}
          icon={c.id === active.id ? <Check size={16} /> : <StatusDot tone={statusTone(c.status)} />}
          onSelect={() => {
            if (c.id !== active.id) void switchTo(c.id);
          }}
        >
          <span className="conn-menu-item">
            <span className="truncate">{c.name}</span>
            <span className="mono muted">{c.accountIdShort || shortId(c.accountId)}</span>
          </span>
        </MenuItem>
      ))}
      <MenuSeparator />
      <MenuItem icon={<Plus size={16} />} onSelect={() => openConnect({ mode: "new", showHelp: false })}>
        Add connection
      </MenuItem>
      <MenuItem icon={<SlidersHorizontal size={16} />} onSelect={() => openSettings("connections")}>
        Manage connections
      </MenuItem>
    </Menu>
  );
}

const MAX_BUCKETS = 8;

function BucketGroup() {
  const connection = useActiveConnection();
  const { rows } = useBuckets(connection);
  const loc = useNavStore((s) => s.loc);
  const openBucket = useNavStore((s) => s.openBucket);
  const [open, setOpen] = useState(true);
  const [showAll, setShowAll] = useState(false);
  if (!connection || rows.length === 0) return null;
  const ordered = [...rows.filter((r) => r.favorite), ...rows.filter((r) => !r.favorite)];
  const visible = showAll ? ordered : ordered.slice(0, MAX_BUCKETS);
  return (
    <div className="bucket-group">
      <button type="button" className="group-heading" aria-expanded={open} onClick={() => setOpen((v) => !v)}>
        <ChevronRight size={12} aria-hidden="true" className="group-chevron" />
        Buckets
      </button>
      {open && (
        <ul className="bucket-links">
          {visible.map((b) => {
            const active = loc.view === "browser" && loc.bucket === b.name;
            return (
              <li key={b.name}>
                <button
                  type="button"
                  className={active ? "bucket-link is-active" : "bucket-link"}
                  aria-current={active ? "page" : undefined}
                  onClick={() => openBucket(b.name)}
                  title={b.name}
                >
                  <span className="truncate">{b.name}</span>
                  {b.favorite && <Star size={11} className="bucket-star" aria-label="Favorite" />}
                </button>
              </li>
            );
          })}
          {ordered.length > MAX_BUCKETS && (
            <li>
              <button type="button" className="bucket-link show-all" onClick={() => setShowAll((v) => !v)}>
                {showAll ? "Show fewer" : `Show all (${ordered.length})`}
              </button>
            </li>
          )}
        </ul>
      )}
    </div>
  );
}

function useNow(intervalMs: number): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const t = window.setInterval(() => setNow(new Date()), intervalMs);
    return () => window.clearInterval(t);
  }, [intervalMs]);
  return now;
}

function StorageSummary() {
  const connection = useActiveConnection();
  const loc = useNavStore((s) => s.loc);
  const client = useQueryClient();
  const [busy, setBusy] = useState(false);
  const now = useNow(30_000);
  const bucket = loc.view === "browser" ? loc.bucket : null;
  const connId = connection?.id ?? "";
  const key = qk.prefixSize(connId, bucket ?? "", loc.prefix);
  const { data } = useQuery<PrefixSize>({
    queryKey: key,
    queryFn: () => prefixSize(connId, bucket ?? "", loc.prefix),
    enabled: false,
    staleTime: Infinity,
    gcTime: Infinity,
  });
  if (!connection || !bucket) return null;

  const calculate = async () => {
    setBusy(true);
    try {
      const size = await prefixSize(connection.id, bucket, loc.prefix);
      client.setQueryData(key, size);
    } catch (e) {
      toast.danger(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const where = folderLabel(bucket, loc.prefix);
  return (
    <div className="storage-summary">
      <div className="storage-head">
        <span className="small secondary truncate" title={where}>
          {where}
        </span>
        <Tooltip content="Calculating lists every object here and uses R2 operations that may be billed." side="right">
          <button type="button" className="info-btn" aria-label="About size calculation">
            <Info size={13} aria-hidden="true" />
          </button>
        </Tooltip>
      </div>
      {data ? (
        <p className="small secondary num">
          {joinDot([`${formatCount(data.objects)} objects`, formatBytes(data.bytes), `calculated ${formatAgo(data.calculatedAt, now)}`])}
        </p>
      ) : (
        <p className="small muted">Size not calculated</p>
      )}
      <button type="button" className="link-button small" onClick={() => void calculate()} disabled={busy}>
        {busy ? "Calculating..." : data ? "Recalculate" : "Calculate"}
      </button>
    </div>
  );
}

export function Sidebar() {
  const collapsed = useUiStore((s) => s.sidebarCollapsed);
  const setCollapsed = useUiStore((s) => s.setSidebarCollapsed);
  const openConnect = useUiStore((s) => s.openConnect);
  const loc = useNavStore((s) => s.loc);
  const navigate = useNavStore((s) => s.navigate);
  const connection = useActiveConnection();
  const activeTransfers = useTransferStore((s) => {
    let n = 0;
    for (const id of s.order) {
      const j = s.jobs[id];
      if (j && isMoving(j.state)) n += 1;
    }
    return n;
  });
  const hasConnection = connection !== null;
  const go = (view: View) => () => navigate({ view });

  return (
    <nav className={collapsed ? "sidebar is-collapsed" : "sidebar"} aria-label="Sidebar">
      <div className="sidebar-header">
        <LogoMark size={20} />
        {!collapsed && <span className="wordmark">R2Dock</span>}
        <IconButton
          className="collapse-btn"
          label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          icon={collapsed ? <PanelLeftOpen size={16} /> : <PanelLeftClose size={16} />}
          onClick={() => setCollapsed(!collapsed)}
          size="sm"
          tooltipSide="right"
        />
      </div>

      {hasConnection ? (
        <>
          <div className="sidebar-section">
            <ConnectionSwitcher collapsed={collapsed} />
          </div>
          <div className="sidebar-nav">
            <NavItem icon={<Database size={16} />} label="All Buckets" active={loc.view === "buckets"} onClick={go("buckets")} collapsed={collapsed} />
            <NavItem icon={<Clock size={16} />} label="Recent" active={loc.view === "recent"} onClick={go("recent")} collapsed={collapsed} />
            <NavItem icon={<Star size={16} />} label="Favorites" active={loc.view === "favorites"} onClick={go("favorites")} collapsed={collapsed} />
            <NavItem
              icon={<ArrowDownUp size={16} />}
              label="Transfers"
              active={loc.view === "transfers"}
              onClick={go("transfers")}
              collapsed={collapsed}
              badge={activeTransfers}
            />
          </div>
          {!collapsed && <BucketGroup />}
          <div className="spacer" />
          {!collapsed && <StorageSummary />}
        </>
      ) : (
        <>
          <div className="sidebar-nav" aria-disabled="true">
            <NavItem icon={<Database size={16} />} label="All Buckets" onClick={go("buckets")} collapsed={collapsed} disabled />
            <NavItem icon={<Clock size={16} />} label="Recent" onClick={go("recent")} collapsed={collapsed} disabled />
            <NavItem icon={<Star size={16} />} label="Favorites" onClick={go("favorites")} collapsed={collapsed} disabled />
            <NavItem icon={<ArrowDownUp size={16} />} label="Transfers" onClick={go("transfers")} collapsed={collapsed} disabled />
          </div>
          {!collapsed && (
            <div className="sidebar-section">
              <Button variant="primary" className="connect-cta" icon={<Plus size={16} />} onClick={() => openConnect({ mode: "new", showHelp: false })}>
                Connect R2 account
              </Button>
            </div>
          )}
          <div className="spacer" />
        </>
      )}

      <div className="sidebar-nav sidebar-bottom">
        <NavItem
          icon={<Settings size={16} />}
          label="Settings"
          active={loc.view === "settings"}
          onClick={() => useNavStore.getState().openSettings()}
          collapsed={collapsed}
        />
      </div>

      {connection && (
        <div className="sidebar-footer" title={`${connection.name}: ${statusLabel(connection.status)}`}>
          <StatusDot tone={statusTone(connection.status)} />
          {!collapsed && (
            <span className="small secondary truncate">{joinDot([connection.name, statusLabel(connection.status)])}</span>
          )}
        </div>
      )}
    </nav>
  );
}
