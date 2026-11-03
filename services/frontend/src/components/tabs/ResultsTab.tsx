import { useQuery } from "@tanstack/react-query";
import { Fragment, useState } from "react";

import { componentMatches, tenderResults } from "@/api/endpoints";
import { StatusBadge } from "@/components/StatusBadge";
import { formatMoney, formatNumber, formatPercent } from "@/lib/format";

function RequirementChecks({
  componentId,
  matchId,
}: {
  componentId: string;
  matchId: string;
}): JSX.Element {
  const matchesQuery = useQuery({
    queryKey: ["match-details", componentId],
    queryFn: () => componentMatches(componentId),
  });

  if (matchesQuery.isPending) {
    return <p className="px-4 py-2 text-[12px] text-zinc-500">Загрузка проверок…</p>;
  }
  if (matchesQuery.isError) {
    return <p className="px-4 py-2 text-[12px] text-red-700">{matchesQuery.error.message}</p>;
  }

  const details = (matchesQuery.data?.details ?? []).filter((detail) => detail.match_id === matchId);
  if (details.length === 0) {
    return <p className="px-4 py-2 text-[12px] text-zinc-500">Проверки отсутствуют.</p>;
  }

  return (
    <table className="table table-tight bg-zinc-50/60">
      <thead>
        <tr>
          <th>Требование</th>
          <th className="w-40">Статус</th>
          <th className="w-32">Ожидалось</th>
          <th className="w-32">Фактически</th>
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
            <td className="text-[12px] text-zinc-600">{detail.explanation || "—"}</td>
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
    return <p className="text-[13px] text-zinc-500">Загрузка результатов…</p>;
  }
  if (resultsQuery.isError) {
    return <p className="text-[13px] text-red-700">{resultsQuery.error.message}</p>;
  }

  const components = resultsQuery.data ?? [];
  if (components.length === 0) {
    return (
      <div className="panel">
        <p className="empty">Позиций пока нет.</p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {components.map((component) => (
        <div key={component.id} className="panel">
          <div className="panel-head">
            <div className="min-w-0">
              <h3 className="text-[13px] font-medium text-zinc-900">
                <span className="mr-2 tabular-nums text-zinc-400">{component.position_no}</span>
                {component.name}
              </h3>
              <p className="mt-0.5 text-[12px] text-zinc-500">
                {component.quantity} {component.unit} · предложений: {component.matches.length}
              </p>
            </div>
            <StatusBadge status={component.status} />
          </div>

          {component.matches.length === 0 ? (
            <p className="empty">
              Совпадений нет: страницы не найдены или товары не извлечены.
            </p>
          ) : (
            <table className="table">
              <thead>
                <tr>
                  <th className="w-12 text-right">Ранг</th>
                  <th>Товар</th>
                  <th className="w-40">Поставщик</th>
                  <th className="w-28 text-right">Цена</th>
                  <th className="w-20 text-right">Срок</th>
                  <th className="w-20 text-right">Балл</th>
                  <th className="w-40">Вердикт</th>
                  <th className="w-20" />
                </tr>
              </thead>
              <tbody>
                {component.matches.map((match) => (
                  <Fragment key={match.match_id}>
                    <tr>
                      <td className="num text-right text-zinc-400">{match.rank ?? "—"}</td>
                      <td>
                        <p className="font-medium">{match.product_name}</p>
                        <p className="text-[12px] text-zinc-500">
                          {[match.manufacturer, match.model].filter(Boolean).join(" · ") || "—"}
                        </p>
                      </td>
                      <td>
                        {match.supplier_name ?? "—"}
                        {match.supplier_url ? (
                          <a
                            className="ml-1 text-[12px]"
                            href={match.supplier_url}
                            target="_blank"
                            rel="noreferrer noopener"
                          >
                            сайт
                          </a>
                        ) : null}
                      </td>
                      <td className="num whitespace-nowrap text-right">
                        {formatMoney(match.price, match.currency)}
                      </td>
                      <td className="num whitespace-nowrap text-right">
                        {match.lead_time_days !== null ? `${match.lead_time_days} дн.` : "—"}
                      </td>
                      <td className="num text-right">{formatPercent(match.total_score)}</td>
                      <td>
                        <StatusBadge status={match.verdict} />
                      </td>
                      <td className="text-right">
                        <button
                          type="button"
                          className="btn-ghost btn-sm"
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
                          <div className="px-4 py-3">
                            <p className="mb-2 text-[12px] text-zinc-600">
                              Техническое соответствие: {formatNumber(match.technical_score)} ·{" "}
                              {match.explanation || "пояснение отсутствует"}
                            </p>
                            <RequirementChecks
                              componentId={component.id}
                              matchId={match.match_id}
                            />
                            {match.canonical_url ? (
                              <a
                                className="mt-2 inline-block text-[12px]"
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
