# ADR 0004: Frontend libraries

Status: Accepted (2026-10-17)

## Decision

- Radix UI primitives for dialogs, menus, context menus, and tooltips.
- TanStack Virtual for long lists, TanStack Query for listings and details, Zustand for UI and transfer state.
- Plain CSS with design tokens as CSS custom properties. Tailwind was not adopted because it adds a build step without removing real complexity for this UI.
- npm (with `package-lock.json`) instead of pnpm, to avoid an extra tool for contributors.

## Consequences

Accessible primitives for menus and dialogs; the file grid's keyboard model is implemented in app code.
