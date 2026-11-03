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

  const entries = activityQuery.data ?? [];

  return (
    <div className="space-y-6">
      <div className="page-head">
        <div>
          <h1 className="page-title">Журнал событий</h1>
          <p className="page-subtitle">Последние 100 событий по всем тендерам.</p>
        </div>
      </div>

      {activityQuery.isPending ? (
        <p className="text-[13px] text-zinc-500">Загрузка…</p>
      ) : activityQuery.isError ? (
        <p className="text-[13px] text-red-700">{activityQuery.error.message}</p>
      ) : entries.length === 0 ? (
        <div className="panel">
          <p className="empty">Событий пока нет.</p>
        </div>
      ) : (
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
                {entry.tender_id ? (
                  <Link
                    className="ml-auto text-[12px] text-zinc-500 no-underline hover:text-zinc-900"
                    to={`/tenders/${entry.tender_id}`}
                  >
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
