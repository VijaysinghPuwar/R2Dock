import * as DM from "@radix-ui/react-dropdown-menu";
import * as CM from "@radix-ui/react-context-menu";
import { Check } from "lucide-react";
import type { ReactNode } from "react";
import "./components.css";

export interface MenuProps {
  trigger: ReactNode;
  children: ReactNode;
  align?: "start" | "center" | "end";
  side?: "top" | "bottom" | "left" | "right";
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}

/** Dropdown menu. `trigger` must be a single focusable element. */
export function Menu({ trigger, children, align = "start", side = "bottom", open, onOpenChange }: MenuProps) {
  return (
    <DM.Root open={open} onOpenChange={onOpenChange} modal={false}>
      <DM.Trigger asChild>{trigger}</DM.Trigger>
      <DM.Portal>
        <DM.Content className="menu" align={align} side={side} sideOffset={4} collisionPadding={8} loop>
          {children}
        </DM.Content>
      </DM.Portal>
    </DM.Root>
  );
}

interface ItemContentProps {
  icon?: ReactNode;
  children: ReactNode;
  shortcut?: string;
}

function ItemContent({ icon, children, shortcut }: ItemContentProps) {
  return (
    <>
      <span className="menu-icon" aria-hidden="true">
        {icon}
      </span>
      <span className="menu-label">{children}</span>
      {shortcut && <span className="menu-shortcut">{shortcut}</span>}
    </>
  );
}

export interface MenuItemProps extends ItemContentProps {
  onSelect?: () => void;
  disabled?: boolean;
  danger?: boolean;
  title?: string;
}

export function MenuItem({ icon, children, shortcut, onSelect, disabled, danger, title }: MenuItemProps) {
  return (
    <DM.Item className={danger ? "menu-item is-danger" : "menu-item"} onSelect={onSelect} disabled={disabled} title={title}>
      <ItemContent icon={icon} shortcut={shortcut}>
        {children}
      </ItemContent>
    </DM.Item>
  );
}

export function MenuSeparator() {
  return <DM.Separator className="menu-separator" />;
}

export function MenuLabel({ children }: { children: ReactNode }) {
  return <DM.Label className="menu-group-label">{children}</DM.Label>;
}

export function MenuCheckboxItem({
  checked,
  onCheckedChange,
  children,
  shortcut,
}: {
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  children: ReactNode;
  shortcut?: string;
}) {
  return (
    <DM.CheckboxItem
      className="menu-item"
      checked={checked}
      onCheckedChange={(v) => onCheckedChange(v === true)}
      onSelect={(e) => e.preventDefault()}
    >
      <ItemContent icon={checked ? <Check size={16} /> : null} shortcut={shortcut}>
        {children}
      </ItemContent>
    </DM.CheckboxItem>
  );
}

export function MenuRadioGroup<T extends string>({
  value,
  onValueChange,
  options,
}: {
  value: T;
  onValueChange: (value: T) => void;
  options: Array<{ value: T; label: string }>;
}) {
  return (
    <DM.RadioGroup value={value} onValueChange={(v) => onValueChange(v as T)}>
      {options.map((o) => (
        <DM.RadioItem key={o.value} value={o.value} className="menu-item" onSelect={(e) => e.preventDefault()}>
          <ItemContent icon={value === o.value ? <Check size={16} /> : null}>{o.label}</ItemContent>
        </DM.RadioItem>
      ))}
    </DM.RadioGroup>
  );
}

/* Context menu */

export interface ContextMenuProps {
  children: ReactNode;
  content: ReactNode;
  onOpenChange?: (open: boolean) => void;
  disabled?: boolean;
}

/** Right-click menu around a region. `children` is the trigger area. */
export function ContextMenu({ children, content, onOpenChange, disabled }: ContextMenuProps) {
  return (
    <CM.Root onOpenChange={onOpenChange} modal={false}>
      <CM.Trigger asChild disabled={disabled}>
        {children}
      </CM.Trigger>
      <CM.Portal>
        <CM.Content className="menu" collisionPadding={8} loop>
          {content}
        </CM.Content>
      </CM.Portal>
    </CM.Root>
  );
}

export function ContextMenuItem({ icon, children, shortcut, onSelect, disabled, danger, title }: MenuItemProps) {
  return (
    <CM.Item className={danger ? "menu-item is-danger" : "menu-item"} onSelect={onSelect} disabled={disabled} title={title}>
      <ItemContent icon={icon} shortcut={shortcut}>
        {children}
      </ItemContent>
    </CM.Item>
  );
}

export function ContextMenuSeparator() {
  return <CM.Separator className="menu-separator" />;
}
