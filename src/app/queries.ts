import { QueryClient, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  appInfo,
  bucketList,
  connectionList,
  errorMessage,
  settingsGet,
  settingsSet,
} from "../ipc/commands";
import type { AppInfo } from "../ipc/bindings/AppInfo";
import type { ConnectionSummary } from "../ipc/bindings/ConnectionSummary";
import type { Preferences } from "../ipc/bindings/Preferences";
import type { SettingsView } from "../ipc/bindings/SettingsView";
import type { BucketEntry } from "../ipc/bindings/BucketEntry";
import { toast } from "../components/toastStore";
import { guessPlatform } from "../lib/platform";

export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        refetchOnWindowFocus: false,
        staleTime: 30_000,
      },
      mutations: { retry: false },
    },
  });
}

export const qk = {
  appInfo: ["appInfo"] as const,
  connections: ["connections"] as const,
  settings: ["settings"] as const,
  buckets: (connectionId: string) => ["buckets", connectionId] as const,
  objectsAll: (connectionId: string) => ["objects", connectionId] as const,
  objectsBucket: (connectionId: string, bucket: string) => ["objects", connectionId, bucket] as const,
  objects: (connectionId: string, bucket: string, prefix: string) => ["objects", connectionId, bucket, prefix] as const,
  search: (connectionId: string, bucket: string, prefix: string) => ["search", connectionId, bucket, prefix] as const,
  head: (connectionId: string, bucket: string, key: string) => ["head", connectionId, bucket, key] as const,
  prefixSize: (connectionId: string, bucket: string, prefix: string) => ["prefixSize", connectionId, bucket, prefix] as const,
  media: (connectionId: string, bucket: string, key: string) => ["media", connectionId, bucket, key] as const,
  text: (connectionId: string, bucket: string, key: string) => ["text", connectionId, bucket, key] as const,
  history: ["history"] as const,
};

export const DEFAULT_PREFERENCES: Preferences = {
  theme: "dark",
  density: "comfortable",
  textSize: "default",
  showThumbnails: true,
  defaultView: "list",
  showDetailsPanel: true,
  concurrency: 3,
  askWhereToSave: false,
  conflictPolicy: "ask",
  historyDays: 30,
  multipartThresholdMib: 100,
  typedConfirmThreshold: 50,
  warnExecutables: true,
  notifyOnComplete: true,
  notifyOnFailure: true,
  logLevel: "standard",
  includeHiddenOsFiles: false,
  renameNoticeSeen: false,
  activityLog: true,
  activityIncludeNames: false,
  activityDays: 14,
};

const FALLBACK_INFO: AppInfo = {
  name: "R2Dock",
  version: "",
  platform: guessPlatform(),
  credentialStore: "system keychain",
  credentialStoreAvailable: true,
};

export function useAppInfo() {
  return useQuery({ queryKey: qk.appInfo, queryFn: appInfo, staleTime: Infinity });
}

/** App info with a safe fallback while it loads. */
export function useAppInfoValue(): AppInfo {
  return useAppInfo().data ?? FALLBACK_INFO;
}

export function usePlatform() {
  return useAppInfoValue().platform;
}

export function useConnections() {
  return useQuery({ queryKey: qk.connections, queryFn: connectionList, staleTime: 60_000 });
}

/** The active connection, or the first one when none is flagged active. */
export function useActiveConnection(): ConnectionSummary | null {
  const { data } = useConnections();
  if (!data || data.length === 0) return null;
  return data.find((c) => c.active) ?? data[0] ?? null;
}

export function useSettings() {
  return useQuery({ queryKey: qk.settings, queryFn: settingsGet, staleTime: Infinity });
}

export function usePreferences(): Preferences {
  return useSettings().data?.preferences ?? DEFAULT_PREFERENCES;
}

/** Change one or more preferences. Applies immediately, then saves. */
export function useUpdatePreferences() {
  const client = useQueryClient();
  const mutation = useMutation({
    mutationFn: (prefs: Preferences) => settingsSet(prefs),
    onMutate: async (prefs) => {
      await client.cancelQueries({ queryKey: qk.settings });
      const previous = client.getQueryData<SettingsView>(qk.settings);
      if (previous) client.setQueryData<SettingsView>(qk.settings, { ...previous, preferences: prefs });
      return { previous };
    },
    onError: (err, _prefs, ctx) => {
      if (ctx?.previous) client.setQueryData(qk.settings, ctx.previous);
      toast.danger(errorMessage(err));
    },
    onSuccess: (view) => client.setQueryData(qk.settings, view),
  });
  return (patch: Partial<Preferences>) => {
    const current = client.getQueryData<SettingsView>(qk.settings)?.preferences ?? DEFAULT_PREFERENCES;
    mutation.mutate({ ...current, ...patch });
  };
}

export interface BucketRow extends BucketEntry {
  favorite: boolean;
  manual: boolean;
}

/** Buckets of a connection merged with manually added ones. */
export function useBuckets(connection: ConnectionSummary | null) {
  const id = connection?.id ?? "";
  const query = useQuery({
    queryKey: qk.buckets(id),
    queryFn: () => bucketList(id),
    enabled: !!connection,
    staleTime: 5 * 60_000,
  });
  const rows: BucketRow[] = [];
  if (connection) {
    const seen = new Set<string>();
    for (const b of query.data ?? []) {
      if (seen.has(b.name)) continue;
      seen.add(b.name);
      rows.push({ ...b, favorite: connection.favoriteBuckets.includes(b.name), manual: connection.manualBuckets.includes(b.name) });
    }
    for (const name of connection.manualBuckets) {
      if (seen.has(name)) continue;
      seen.add(name);
      rows.push({ name, created: null, favorite: connection.favoriteBuckets.includes(name), manual: true });
    }
  }
  // Without a successful listing, manual buckets are still usable.
  const usable = rows.length > 0;
  return { ...query, rows, usable };
}
