import { useQuery } from "@tanstack/react-query";
import { Fragment, useState } from "react";

import { componentMatches, tenderResults } from "@/api/endpoints";
import { StatusBadge } from "@/components/StatusBadge";
import { formatMoney, formatNumber, formatPercent } from "@/lib/format";

function RequirementChecks({ componentId, matchId }: { componentId: string; matchId: string }): JSX.Element {
  const matchesQuery = useQuery({
    queryKey: ["match-details", componentId],
    queryFn: () => componentMatches(componentId),
  });

  if (matchesQuery.isPending) {
    return <p className="px-3 py-2 text-xs text-slate-500">Загрузка проверок…</p>;
  }
  if (matchesQuery.isError) {
    return <p className="px-3 py-2 text-xs text-red-600">{matchesQuery.error.message}</p>;
  }

  const details = (matchesQuery.data?.details ?? []).filter((detail) => detail.match_id === matchId);
  if (details.length === 0) {
    return <p className="px-3 py-2 text-xs text-slate-500">Проверки отсутствуют.</p>;
  }

  return (
    <table className="table bg-slate-50">
      <thead>
        <tr>
          <th>Требование</th>
          <th>Статус</th>
          <th>Ожидалось</th>
          <th>Фактически</th>
          <th>Пояснение</th>
        </tr>
      </thead>
      <tbody>
        {details.map((detail, index) => (
          <tr key={`${detail.requirement_name}-${index}`}>
            <td>{detail.requirement_name}</td>
            <td>
              <StatusBadge status={detail.status} />
            </td>
            <td>{detail.expected || "—"}</td>
            <td>
              {detail.actual || "—"} {detail.unit}
            </td>
            <td className="text-xs text-slate-600">{detail.explanation || "—"}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export default function ResultsTab({ tenderId }: { tenderId: string }): JSX.Element {
  const [expanded, setExpanded] = useState<string | null>(null);

  const resultsQuery = useQuery({
    queryKey: ["results", tenderId],
    queryFn: () => tenderResults(tenderId),
    refetchInterval: 10_000,
  });

  if (resultsQuery.isPending) {
    return <p className="text-sm text-slate-500">Загрузка результатов…</p>;
  }
  if (resultsQuery.isError) {
    return <p className="text-sm text-red-600">{resultsQuery.error.message}</p>;
  }

  const components = resultsQuery.data ?? [];
  if (components.length === 0) {
    return <p className="text-sm text-slate-500">Позиций пока нет.</p>;
  }

  return (
    <div className="space-y-4">
      {components.map((component) => (
        <div key={component.id} className="card">
          <div className="flex flex-wrap items-center justify-between gap-3 border-b border-slate-200 p-4">
            <div>
              <h3 className="text-sm font-semibold text-slate-900">
                {component.position_no}. {component.name}
              </h3>
              <p className="mt-0.5 text-xs text-slate-500">
                {component.quantity} {component.unit} · совпадений: {component.matches.length}
              </p>
            </div>
            <StatusBadge status={component.status} />
          </div>

          {component.matches.length === 0 ? (
            <p className="p-4 text-sm text-slate-500">
              Совпадений нет: страницы не найдены или товары не извлечены.
            </p>
          ) : (
            <table className="table">
              <thead>
                <tr>
                  <th>Ранг</th>
                  <th>Товар</th>
                  <th>Поставщик</th>
                  <th>Цена</th>
                  <th>Срок</th>
                  <th>Балл</th>
                  <th>Вердикт</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {component.matches.map((match) => (
                  <Fragment key={match.match_id}>
                    <tr>
                      <td>{match.rank ?? "—"}</td>
                      <td>
                        <p className="font-medium">{match.product_name}</p>
                        <p className="text-xs text-slate-500">
                          {[match.manufacturer, match.model].filter(Boolean).join(" · ") || "—"}
                        </p>
                      </td>
                      <td>
                        {match.supplier_name ?? "—"}
                        {match.supplier_url ? (
                          <a
                            className="ml-1 text-xs"
                            href={match.supplier_url}
                            target="_blank"
                            rel="noreferrer noopener"
                          >
                            сайт
                          </a>
                        ) : null}
                      </td>
                      <td className="whitespace-nowrap">
                        {formatMoney(match.price, match.currency)}
                      </td>
                      <td>{match.lead_time_days !== null ? `${match.lead_time_days} дн.` : "—"}</td>
                      <td>{formatPercent(match.total_score)}</td>
                      <td>
                        <StatusBadge status={match.verdict} />
                      </td>
                      <td className="text-right">
                        <button
                          type="button"
                          className="text-xs text-brand-700"
                          onClick={() =>
                            setExpanded(expanded === match.match_id ? null : match.match_id)
                          }
                        >
                          {expanded === match.match_id ? "Скрыть" : "Проверки"}
                        </button>
                      </td>
                    </tr>
                    {expanded === match.match_id ? (
                      <tr>
                        <td colSpan={8} className="p-0">
                          <div className="p-3">
                            <p className="mb-2 text-xs text-slate-600">
                              Техническое соответствие: {formatNumber(match.technical_score)} ·{" "}
                              {match.explanation || "пояснение отсутствует"}
                            </p>
                            <RequirementChecks
                              componentId={component.id}
                              matchId={match.match_id}
                            />
                            {match.canonical_url ? (
                              <a
                                className="mt-2 inline-block text-xs"
                                href={match.canonical_url}
                                target="_blank"
                                rel="noreferrer noopener"
                              >
                                Открыть страницу товара
                              </a>
                            ) : null}
                          </div>
                        </td>
                      </tr>
                    ) : null}
                  </Fragment>
                ))}
              </tbody>
            </table>
          )}
        </div>
      ))}
    </div>
  );
}
