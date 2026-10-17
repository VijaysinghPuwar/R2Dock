import * as RadixTooltip from "@radix-ui/react-tooltip";
import type { ReactNode } from "react";
import "./components.css";

export interface TooltipProps {
  content: ReactNode;
  children: ReactNode;
  shortcut?: string;
  side?: "top" | "bottom" | "left" | "right";
}

/** Tooltip with a 500 ms delay. Wraps a single focusable child. */
export function Tooltip({ content, children, shortcut, side = "bottom" }: TooltipProps) {
  return (
    <RadixTooltip.Root>
      <RadixTooltip.Trigger asChild>{children}</RadixTooltip.Trigger>
      <RadixTooltip.Portal>
        <RadixTooltip.Content className="tooltip" side={side} sideOffset={6} collisionPadding={8}>
          {content}
          {shortcut && <kbd className="kbd kbd-inline">{shortcut}</kbd>}
        </RadixTooltip.Content>
      </RadixTooltip.Portal>
    </RadixTooltip.Root>
  );
}

export function TooltipProvider({ children }: { children: ReactNode }) {
  return (
    <RadixTooltip.Provider delayDuration={500} skipDelayDuration={200}>
      {children}
    </RadixTooltip.Provider>
  );
}
