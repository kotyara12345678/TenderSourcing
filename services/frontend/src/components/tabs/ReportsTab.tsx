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
    return <p className="text-sm text-slate-500">Загрузка отчётов…</p>;
  }
  if (reportsQuery.isError) {
    return <p className="text-sm text-red-600">{reportsQuery.error.message}</p>;
  }

  const reports = reportsQuery.data ?? [];
  if (reports.length === 0) {
    return (
      <div className="card p-6 text-sm text-slate-500">
        Отчёты появятся после завершения поиска и сопоставления.
      </div>
    );
  }

  return (
    <div className="card">
      <table className="table">
        <thead>
          <tr>
            <th>Формат</th>
            <th>Размер</th>
            <th>Статус</th>
            <th>Создан</th>
            <th>Готов</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {reports.map((report) => (
            <tr key={report.id}>
              <td className="uppercase">{report.format}</td>
              <td>{formatBytes(report.size_bytes)}</td>
              <td>
                <StatusBadge status={report.status} />
                {report.error_message ? (
                  <p className="mt-0.5 text-xs text-red-600">{report.error_message}</p>
                ) : null}
              </td>
              <td className="whitespace-nowrap text-slate-500">
                {formatDateTime(report.created_at)}
              </td>
              <td className="whitespace-nowrap text-slate-500">
                {formatDateTime(report.ready_at)}
              </td>
              <td className="text-right">
                {report.status === "READY" ? (
                  <a
                    className="btn-secondary"
                    href={downloadUrl(`/reports/${report.id}/download`)}
                    rel="noreferrer"
                  >
                    Скачать
                  </a>
                ) : (
                  <span className="text-xs text-slate-400">недоступен</span>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
