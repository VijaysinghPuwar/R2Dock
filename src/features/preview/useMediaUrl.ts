import { useQuery } from "@tanstack/react-query";
import { mediaToken } from "../../ipc/commands";
import { qk } from "../../app/queries";
import { mediaUrl } from "../../lib/media";

/** A media scheme URL for an image or audio object. Never a presigned URL. */
export function useMediaUrl(connectionId: string, bucket: string, key: string, enabled = true) {
  const query = useQuery({
    queryKey: qk.media(connectionId, bucket, key),
    queryFn: () => mediaToken(connectionId, bucket, key),
    enabled: enabled && !!connectionId && !!bucket && !!key,
    staleTime: 4 * 60_000,
    gcTime: 5 * 60_000,
  });
  return { ...query, url: query.data ? mediaUrl(query.data) : null };
}
