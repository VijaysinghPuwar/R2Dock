import { useQueryClient } from "@tanstack/react-query";
import { Database, Star } from "lucide-react";
import { useNavStore } from "../../app/navStore";
import { qk, useActiveConnection } from "../../app/queries";
import { IconButton } from "../../components/Button";
import { EmptyState } from "../../components/Feedback";
import { toast } from "../../components/toastStore";
import { connectionToggleFavorite, errorMessage } from "../../ipc/commands";
import "./views.css";

export function FavoritesView() {
  const client = useQueryClient();
  const connection = useActiveConnection();
  const openBucket = useNavStore((s) => s.openBucket);
  const favorites = connection?.favoriteBuckets ?? [];

  const unstar = async (bucket: string) => {
    if (!connection) return;
    try {
      await connectionToggleFavorite(connection.id, bucket);
      await client.invalidateQueries({ queryKey: qk.connections });
    } catch (e) {
      toast.danger(errorMessage(e));
    }
  };

  return (
    <div className="page">
      <div className="page-header">
        <h1 className="page-title">Favorites</h1>
        {connection && <span className="page-subtitle">{connection.name}</span>}
      </div>
      {favorites.length === 0 ? (
        <EmptyState icon={<Star size={32} />} title="No favorites yet" body="Star a bucket in All Buckets to pin it here." />
      ) : (
        <ul className="view-list">
          {favorites.map((bucket) => (
            <li key={bucket} className="fav-row">
              <button type="button" className="fav-open" onClick={() => openBucket(bucket)}>
                <Database size={16} className="secondary" aria-hidden="true" />
                <span className="truncate">{bucket}</span>
              </button>
              <IconButton
                label={`Remove ${bucket} from favorites`}
                size="sm"
                className="fav-star"
                icon={<Star size={14} fill="currentColor" />}
                onClick={() => void unstar(bucket)}
              />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
