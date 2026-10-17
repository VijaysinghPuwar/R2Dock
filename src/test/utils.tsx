import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, type RenderOptions } from "@testing-library/react";
import type { ReactElement } from "react";
import { TooltipProvider } from "../components/Tooltip";

export function testQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity }, mutations: { retry: false } },
  });
}

/** Render inside the providers the app uses. */
export function renderWithProviders(ui: ReactElement, options: RenderOptions & { client?: QueryClient } = {}) {
  const client = options.client ?? testQueryClient();
  const result = render(
    <QueryClientProvider client={client}>
      <TooltipProvider>{ui}</TooltipProvider>
    </QueryClientProvider>,
    options,
  );
  return { ...result, client };
}
