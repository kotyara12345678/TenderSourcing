import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router-dom";

import { recentActivity } from "@/api/endpoints";
import { formatDateTime } from "@/lib/format";

export default function ActivityPage(): JSX.Element {
  const activityQuery = useQuery({
    queryKey: ["activity", "global"],
    queryFn: () => recentActivity(100),
    refetchInterval: 15_000,
  });

  return (
    <div className="space-y-4">
      <h1 className="text-xl font-semibold text-slate-900">Журнал событий</h1>
      {activityQuery.isPending ? (
        <p className="text-sm text-slate-500">Загрузка…</p>
      ) : activityQuery.isError ? (
        <p className="text-sm text-red-600">{activityQuery.error.message}</p>
      ) : (activityQuery.data ?? []).length === 0 ? (
        <div className="card p-6 text-sm text-slate-500">Событий пока нет.</div>
      ) : (
        <div className="card">
          <ul className="divide-y divide-slate-100">
            {(activityQuery.data ?? []).map((entry) => (
              <li key={entry.id} className="flex flex-wrap items-baseline gap-3 px-4 py-2.5">
                <span className="w-40 shrink-0 font-mono text-xs text-slate-400">
                  {formatDateTime(entry.created_at)}
                </span>
                <span className="rounded bg-slate-100 px-1.5 py-0.5 font-mono text-[11px] text-slate-600">
                  {entry.event_type}
                </span>
                <span className="text-sm text-slate-700">{entry.message}</span>
                {entry.tender_id ? (
                  <Link className="ml-auto text-xs" to={`/tenders/${entry.tender_id}`}>
                    открыть тендер
                  </Link>
                ) : null}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
