import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Link, useParams } from "react-router-dom";

import { cancelTender, getTender, startSearch } from "@/api/endpoints";
import { isTerminalStatus, StatusBadge } from "@/components/StatusBadge";
import ActivityTab from "@/components/tabs/ActivityTab";
import ComponentsTab from "@/components/tabs/ComponentsTab";
import DocumentsTab from "@/components/tabs/DocumentsTab";
import ReportsTab from "@/components/tabs/ReportsTab";
import ResultsTab from "@/components/tabs/ResultsTab";
import { useProgressSocket } from "@/hooks/useProgressSocket";
import type { ProgressMessage } from "@/hooks/useProgressSocket";
import { formatDateTime } from "@/lib/format";

type TabKey = "documents" | "components" | "results" | "reports" | "activity";

const TABS: { key: TabKey; label: string }[] = [
  { key: "documents", label: "Документы" },
  { key: "components", label: "Позиции и требования" },
  { key: "results", label: "Результаты" },
  { key: "reports", label: "Отчёты" },
  { key: "activity", label: "Журнал" },
];

const LIVE_FEED_LIMIT = 8;

export default function TenderPage(): JSX.Element {
  const { tenderId } = useParams<{ tenderId: string }>();
  const queryClient = useQueryClient();
  const [tab, setTab] = useState<TabKey>("documents");
  const [feed, setFeed] = useState<ProgressMessage[]>([]);
  const [actionError, setActionError] = useState<string | null>(null);

  const tenderQuery = useQuery({
    queryKey: ["tender", tenderId],
    queryFn: () => getTender(tenderId ?? ""),
    enabled: Boolean(tenderId),
    refetchInterval: 5_000,
  });

  const tender = tenderQuery.data;

  const { connected } = useProgressSocket(tenderId ?? null, (message) => {
    setFeed((current) => [message, ...current].slice(0, LIVE_FEED_LIMIT));
    if (message.type === "progress") {
      void queryClient.invalidateQueries({ queryKey: ["tender", tenderId] });
      void queryClient.invalidateQueries({ queryKey: ["components", tenderId] });
      void queryClient.invalidateQueries({ queryKey: ["documents", tenderId] });
      void queryClient.invalidateQueries({ queryKey: ["reports", tenderId] });
      void queryClient.invalidateQueries({ queryKey: ["results", tenderId] });
      void queryClient.invalidateQueries({ queryKey: ["activity", tenderId] });
    }
  });

  const startMutation = useMutation({
    mutationFn: () => startSearch(tenderId ?? ""),
    onSuccess: () => {
      setActionError(null);
      void queryClient.invalidateQueries({ queryKey: ["tender", tenderId] });
    },
    onError: (error: Error) => setActionError(error.message),
  });

  const cancelMutation = useMutation({
    mutationFn: () => cancelTender(tenderId ?? ""),
    onSuccess: () => {
      setActionError(null);
      void queryClient.invalidateQueries({ queryKey: ["tender", tenderId] });
    },
    onError: (error: Error) => setActionError(error.message),
  });

  const editable = useMemo(() => {
    if (!tender) return false;
    return !isTerminalStatus(tender.status);
  }, [tender]);

  if (!tenderId) {
    return <p className="text-sm text-red-600">Некорректный адрес тендера.</p>;
  }

  if (tenderQuery.isPending) {
    return <p className="text-sm text-slate-500">Загрузка тендера…</p>;
  }

  if (tenderQuery.isError || !tender) {
    return <p className="text-sm text-red-600">{tenderQuery.error?.message ?? "Тендер не найден"}</p>;
  }

  const canStartSearch = tender.status === "REQUIREMENTS_REVIEW";

  return (
    <div className="space-y-6">
      <section className="card p-6">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div>
            <Link to="/tenders" className="text-xs text-slate-500">
              ← Все тендеры
            </Link>
            <h1 className="mt-1 text-xl font-semibold text-slate-900">{tender.name}</h1>
            {tender.description ? (
              <p className="mt-1 max-w-3xl text-sm text-slate-600">{tender.description}</p>
            ) : null}
            <div className="mt-3 flex flex-wrap items-center gap-3 text-xs text-slate-500">
              <StatusBadge status={tender.status} />
              <span>Позиций: {tender.components_count}</span>
              <span>Документов: {tender.documents_count}</span>
              <span>Отчётов: {tender.reports_count}</span>
              <span>Обновлён: {formatDateTime(tender.updated_at)}</span>
              <span className={connected ? "text-emerald-600" : "text-slate-400"}>
                {connected ? "● живое обновление" : "○ офлайн"}
              </span>
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <button
              type="button"
              className="btn-primary"
              disabled={!canStartSearch || startMutation.isPending}
              title={
                canStartSearch
                  ? "Запустить поиск по всем позициям"
                  : "Поиск можно запустить после проверки требований"
              }
              onClick={() => startMutation.mutate()}
            >
              {startMutation.isPending ? "Запуск…" : "Запустить поиск"}
            </button>
            <button
              type="button"
              className="btn-danger"
              disabled={!editable || cancelMutation.isPending}
              onClick={() => cancelMutation.mutate()}
            >
              Отменить
            </button>
          </div>
        </div>

        {tender.error_message ? (
          <p className="mt-4 rounded-lg bg-red-50 px-3 py-2 text-sm text-red-700">
            {tender.error_message}
          </p>
        ) : null}
        {actionError ? (
          <p className="mt-4 rounded-lg bg-amber-50 px-3 py-2 text-sm text-amber-800">{actionError}</p>
        ) : null}

        {tender.latest_run ? (
          <dl className="mt-4 grid grid-cols-2 gap-3 text-xs text-slate-600 sm:grid-cols-3 lg:grid-cols-6">
            <div>
              <dt className="text-slate-400">Запуск</dt>
              <dd>
                <StatusBadge status={tender.latest_run.status} />
              </dd>
            </div>
            <div>
              <dt className="text-slate-400">Позиции</dt>
              <dd>
                {tender.latest_run.components_done}/{tender.latest_run.components_total}
                {tender.latest_run.components_failed > 0
                  ? ` (ошибок: ${tender.latest_run.components_failed})`
                  : ""}
              </dd>
            </div>
            <div>
              <dt className="text-slate-400">Запросов</dt>
              <dd>{tender.latest_run.queries_issued}</dd>
            </div>
            <div>
              <dt className="text-slate-400">Найдено</dt>
              <dd>{tender.latest_run.results_found}</dd>
            </div>
            <div>
              <dt className="text-slate-400">Страниц</dt>
              <dd>{tender.latest_run.pages_fetched}</dd>
            </div>
            <div>
              <dt className="text-slate-400">Начат</dt>
              <dd>{formatDateTime(tender.latest_run.started_at)}</dd>
            </div>
          </dl>
        ) : null}

        {feed.length > 0 ? (
          <div className="mt-4 max-h-40 space-y-1 overflow-y-auto rounded-lg bg-slate-50 p-3 font-mono text-xs text-slate-600">
            {feed.map((message, index) => (
              <p key={`${message.event_type ?? message.type}-${index}`}>
                <span className="text-slate-400">{message.stage ?? message.type}</span>{" "}
                {message.message ?? ""}
              </p>
            ))}
          </div>
        ) : null}
      </section>

      <nav className="flex flex-wrap gap-1">
        {TABS.map((item) => (
          <button
            key={item.key}
            type="button"
            onClick={() => setTab(item.key)}
            className={`rounded-lg px-3 py-1.5 text-sm font-medium transition ${
              tab === item.key
                ? "bg-white text-brand-700 shadow-sm ring-1 ring-slate-200"
                : "text-slate-600 hover:bg-white/70"
            }`}
          >
            {item.label}
          </button>
        ))}
      </nav>

      <section>
        {tab === "documents" ? <DocumentsTab tenderId={tenderId} /> : null}
        {tab === "components" ? <ComponentsTab tenderId={tenderId} /> : null}
        {tab === "results" ? <ResultsTab tenderId={tenderId} /> : null}
        {tab === "reports" ? <ReportsTab tenderId={tenderId} /> : null}
        {tab === "activity" ? <ActivityTab tenderId={tenderId} /> : null}
      </section>
    </div>
  );
}
