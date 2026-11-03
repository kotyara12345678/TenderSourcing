import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router-dom";

import { usageSummary } from "@/api/endpoints";
import { formatDateTime, formatMoney, formatNumber } from "@/lib/format";

export default function UsagePage(): JSX.Element {
  const usageQuery = useQuery({
    queryKey: ["usage"],
    queryFn: () => usageSummary(),
    refetchInterval: 30_000,
  });

  if (usageQuery.isPending) {
    return <p className="text-sm text-slate-500">Загрузка…</p>;
  }
  if (usageQuery.isError) {
    return <p className="text-sm text-red-600">{usageQuery.error.message}</p>;
  }

  const usage = usageQuery.data;
  if (!usage) {
    return <p className="text-sm text-slate-500">Данных нет.</p>;
  }

  return (
    <div className="space-y-6">
      <h1 className="text-xl font-semibold text-slate-900">Расходы внешних API</h1>

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <div className="card p-4">
          <p className="text-xs uppercase tracking-wide text-slate-400">Вызовов</p>
          <p className="mt-1 text-2xl font-semibold">{formatNumber(usage.totals.calls)}</p>
        </div>
        <div className="card p-4">
          <p className="text-xs uppercase tracking-wide text-slate-400">Кредиты Keenable</p>
          <p className="mt-1 text-2xl font-semibold">{formatNumber(usage.totals.credits)}</p>
        </div>
        <div className="card p-4">
          <p className="text-xs uppercase tracking-wide text-slate-400">Токены LLM</p>
          <p className="mt-1 text-2xl font-semibold">
            {formatNumber(usage.totals.tokens_in + usage.totals.tokens_out)}
          </p>
          <p className="text-xs text-slate-500">
            вход {formatNumber(usage.totals.tokens_in)} · выход {formatNumber(usage.totals.tokens_out)}
          </p>
        </div>
        <div className="card p-4">
          <p className="text-xs uppercase tracking-wide text-slate-400">Стоимость</p>
          <p className="mt-1 text-2xl font-semibold">{formatMoney(usage.totals.cost_usd, "USD")}</p>
          <p className="text-xs text-slate-500">учитывается только при наличии тарифа</p>
        </div>
      </div>

      <div className="grid gap-4 lg:grid-cols-2">
        <div className="card">
          <h2 className="border-b border-slate-200 p-4 text-sm font-semibold">По сервисам</h2>
          <table className="table">
            <thead>
              <tr>
                <th>Сервис</th>
                <th>Вызовов</th>
                <th>Кредиты</th>
                <th>Токены</th>
              </tr>
            </thead>
            <tbody>
              {usage.by_service.map((service) => (
                <tr key={service.service}>
                  <td className="font-mono text-xs">{service.service}</td>
                  <td>{formatNumber(service.calls)}</td>
                  <td>{formatNumber(service.credits)}</td>
                  <td>{formatNumber(service.tokens_in + service.tokens_out)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <div className="card">
          <h2 className="border-b border-slate-200 p-4 text-sm font-semibold">По тендерам</h2>
          <table className="table">
            <thead>
              <tr>
                <th>Тендер</th>
                <th>Вызовов</th>
                <th>Кредиты</th>
              </tr>
            </thead>
            <tbody>
              {usage.by_tender.length === 0 ? (
                <tr>
                  <td colSpan={3} className="text-slate-500">
                    Данных нет.
                  </td>
                </tr>
              ) : (
                usage.by_tender.map((tender) => (
                  <tr key={tender.tender_id}>
                    <td>
                      <Link to={`/tenders/${tender.tender_id}`}>{tender.tender_name}</Link>
                    </td>
                    <td>{formatNumber(tender.calls)}</td>
                    <td>{formatNumber(tender.credits)}</td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </div>

      <div className="card">
        <h2 className="border-b border-slate-200 p-4 text-sm font-semibold">Последние вызовы</h2>
        <table className="table">
          <thead>
            <tr>
              <th>Когда</th>
              <th>Сервис</th>
              <th>SKU</th>
              <th>Количество</th>
              <th>Кредиты</th>
            </tr>
          </thead>
          <tbody>
            {usage.recent.map((entry) => (
              <tr key={entry.id}>
                <td className="whitespace-nowrap text-slate-500">{formatDateTime(entry.created_at)}</td>
                <td className="font-mono text-xs">{entry.service}</td>
                <td className="font-mono text-xs">{entry.sku || "—"}</td>
                <td>{formatNumber(entry.quantity)}</td>
                <td>{formatNumber(entry.credits)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
