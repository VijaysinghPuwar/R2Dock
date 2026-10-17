import * as RadixDialog from "@radix-ui/react-dialog";
import { X } from "lucide-react";
import type { ReactNode } from "react";
import { IconButton } from "./Button";
import "./components.css";

export type DialogWidth = "confirm" | "form" | "review" | "wide";

export interface DialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: ReactNode;
  /** Optional description; used as the accessible description. */
  description?: ReactNode;
  children?: ReactNode;
  footer?: ReactNode;
  width?: DialogWidth;
  /** Prevents closing with Esc or the backdrop (for running destructive operations). */
  locked?: boolean;
  className?: string;
  hideClose?: boolean;
}

export function Dialog({
  open,
  onOpenChange,
  title,
  description,
  children,
  footer,
  width = "form",
  locked = false,
  className,
  hideClose = false,
}: DialogProps) {
  return (
    <RadixDialog.Root open={open} onOpenChange={(next) => (!next && locked ? undefined : onOpenChange(next))}>
      <RadixDialog.Portal>
        <RadixDialog.Overlay className="dialog-overlay" />
        <RadixDialog.Content
          className={["dialog", `dialog-${width}`, className].filter(Boolean).join(" ")}
          {...(description ? {} : { "aria-describedby": undefined })}
          tabIndex={-1}
          onOpenAutoFocus={(e) => {
            // Focus the first field, or the dialog itself, instead of the Close button and its tooltip.
            e.preventDefault();
            const root = e.currentTarget as HTMLElement | null;
            if (root && document.activeElement && root.contains(document.activeElement) && document.activeElement !== root) return;
            const field = root?.querySelector<HTMLElement>(
              "input:not([type=hidden]):not([disabled]), textarea:not([disabled]), select:not([disabled])",
            );
            (field ?? root)?.focus();
          }}
          onEscapeKeyDown={(e) => {
            if (locked) e.preventDefault();
          }}
          onPointerDownOutside={(e) => {
            if (locked) e.preventDefault();
          }}
          onInteractOutside={(e) => {
            if (locked) e.preventDefault();
          }}
        >
          <div className="dialog-header">
            <RadixDialog.Title className="dialog-title">{title}</RadixDialog.Title>
            {!hideClose && (
              <RadixDialog.Close asChild>
                <IconButton label="Close" icon={<X size={16} />} size="sm" disabled={locked} />
              </RadixDialog.Close>
            )}
          </div>
          {description && <RadixDialog.Description className="dialog-description">{description}</RadixDialog.Description>}
          {children !== undefined && <div className="dialog-body">{children}</div>}
          {footer && <div className="dialog-footer">{footer}</div>}
        </RadixDialog.Content>
      </RadixDialog.Portal>
    </RadixDialog.Root>
  );
}
