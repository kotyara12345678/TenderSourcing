import { useQuery } from "@tanstack/react-query";

import { tenderActivity } from "@/api/endpoints";
import { formatDateTime } from "@/lib/format";

export default function ActivityTab({ tenderId }: { tenderId: string }): JSX.Element {
  const activityQuery = useQuery({
    queryKey: ["activity", tenderId],
    queryFn: () => tenderActivity(tenderId, 100),
    refetchInterval: 15_000,
  });

  if (activityQuery.isPending) {
    return <p className="text-[13px] text-zinc-500">Загрузка журнала…</p>;
  }
  if (activityQuery.isError) {
    return <p className="text-[13px] text-red-700">{activityQuery.error.message}</p>;
  }

  const entries = activityQuery.data ?? [];
  if (entries.length === 0) {
    return (
      <div className="panel">
        <p className="empty">Событий пока нет.</p>
      </div>
    );
  }

  return (
    <div className="panel">
      <ul className="divide-y divide-zinc-100">
        {entries.map((entry) => (
          <li key={entry.id} className="flex flex-wrap items-baseline gap-3 px-4 py-2">
            <span className="w-36 shrink-0 font-mono text-[11px] tabular-nums text-zinc-400">
              {formatDateTime(entry.created_at)}
            </span>
            <span className="rounded bg-zinc-100 px-1.5 py-0.5 font-mono text-[11px] text-zinc-600">
              {entry.event_type}
            </span>
            <span className="text-[13px] text-zinc-700">{entry.message}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
