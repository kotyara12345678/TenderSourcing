import { useQuery } from "@tanstack/react-query";

import { downloadUrl } from "@/api/client";
import { listReports } from "@/api/endpoints";
import { StatusBadge } from "@/components/StatusBadge";
import { formatBytes, formatDateTime } from "@/lib/format";

export default function ReportsTab({ tenderId }: { tenderId: string }): JSX.Element {
  const reportsQuery = useQuery({
    queryKey: ["reports", tenderId],
    queryFn: () => listReports(tenderId),
    refetchInterval: 10_000,
  });

  if (reportsQuery.isPending) {
    return <p className="text-[13px] text-zinc-500">Загрузка отчётов…</p>;
  }
  if (reportsQuery.isError) {
    return <p className="text-[13px] text-red-700">{reportsQuery.error.message}</p>;
  }

  const reports = reportsQuery.data ?? [];
  if (reports.length === 0) {
    return (
      <div className="panel">
        <p className="empty">Отчёты появятся после завершения поиска и сопоставления.</p>
      </div>
    );
  }

  return (
    <div className="panel">
      <table className="table">
        <thead>
          <tr>
            <th className="w-24">Формат</th>
            <th className="w-28 text-right">Размер</th>
            <th className="w-40">Статус</th>
            <th className="w-40">Создан</th>
            <th className="w-40">Готов</th>
            <th className="w-24" />
          </tr>
        </thead>
        <tbody>
          {reports.map((report) => (
            <tr key={report.id}>
              <td className="mono uppercase">{report.format}</td>
              <td className="num text-right">{formatBytes(report.size_bytes)}</td>
              <td>
                <StatusBadge status={report.status} />
                {report.error_message ? (
                  <p className="mt-0.5 text-[12px] text-red-700">{report.error_message}</p>
                ) : null}
              </td>
              <td className="whitespace-nowrap text-[12px] text-zinc-500">
                {formatDateTime(report.created_at)}
              </td>
              <td className="whitespace-nowrap text-[12px] text-zinc-500">
                {formatDateTime(report.ready_at)}
              </td>
              <td className="text-right">
                {report.status === "READY" ? (
                  <a
                    className="btn-secondary btn-sm no-underline"
                    href={downloadUrl(`/reports/${report.id}/download`)}
                    rel="noreferrer"
                  >
                    Скачать
                  </a>
                ) : (
                  <span className="text-[12px] text-zinc-400">—</span>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
