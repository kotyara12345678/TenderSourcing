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
    return <p className="text-sm text-slate-500">Загрузка журнала…</p>;
  }
  if (activityQuery.isError) {
    return <p className="text-sm text-red-600">{activityQuery.error.message}</p>;
  }

  const entries = activityQuery.data ?? [];
  if (entries.length === 0) {
    return <div className="card p-6 text-sm text-slate-500">Событий пока нет.</div>;
  }

  return (
    <div className="card">
      <ul className="divide-y divide-slate-100">
        {entries.map((entry) => (
          <li key={entry.id} className="flex flex-wrap items-baseline gap-3 px-4 py-2.5">
            <span className="w-40 shrink-0 font-mono text-xs text-slate-400">
              {formatDateTime(entry.created_at)}
            </span>
            <span className="rounded bg-slate-100 px-1.5 py-0.5 font-mono text-[11px] text-slate-600">
              {entry.event_type}
            </span>
            <span className="text-sm text-slate-700">{entry.message}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
