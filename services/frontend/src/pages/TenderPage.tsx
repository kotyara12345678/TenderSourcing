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
  { key: "components", label: "Позиции" },
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
    // "snapshot" is the state handshake sent on connect, not a feed entry.
    if (message.type === "progress" && (message.stage || message.message)) {
      setFeed((current) => [message, ...current].slice(0, LIVE_FEED_LIMIT));
    }
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
    return <p className="text-[13px] text-red-700">Некорректный адрес тендера.</p>;
  }

  if (tenderQuery.isPending) {
    return <p className="text-[13px] text-zinc-500">Загрузка тендера…</p>;
  }

  if (tenderQuery.isError || !tender) {
    return (
      <p className="text-[13px] text-red-700">{tenderQuery.error?.message ?? "Тендер не найден"}</p>
    );
  }

  const canStartSearch = tender.status === "REQUIREMENTS_REVIEW";
  const run = tender.latest_run;
  const progress =
    run && run.components_total > 0
      ? Math.min(100, Math.round((run.components_done / run.components_total) * 100))
      : 0;

  const counts: Record<TabKey, number | null> = {
    documents: tender.documents_count,
    components: tender.components_count,
    results: null,
    reports: tender.reports_count,
    activity: null,
  };

  return (
    <div className="space-y-6">
      <div>
        <Link to="/tenders" className="text-[12px] text-zinc-500 no-underline hover:text-zinc-900">
          ← Тендеры
        </Link>

        <div className="mt-2 flex flex-wrap items-start justify-between gap-4">
          <div className="min-w-0">
            <h1 className="page-title">{tender.name}</h1>
            {tender.description ? (
              <p className="mt-1 max-w-3xl text-[13px] text-zinc-500">{tender.description}</p>
            ) : null}
            <div className="mt-3 flex flex-wrap items-center gap-x-4 gap-y-2 text-[12px] text-zinc-500">
              <StatusBadge status={tender.status} />
              <span className="tabular-nums">позиций: {tender.components_count}</span>
              <span className="tabular-nums">документов: {tender.documents_count}</span>
              <span className="tabular-nums">отчётов: {tender.reports_count}</span>
              <span>обновлён {formatDateTime(tender.updated_at)}</span>
              <span className="flex items-center gap-1.5">
                <span
                  className={`status-dot ${connected ? "status-dot-live" : ""}`}
                  aria-hidden="true"
                />
                {connected ? "живое обновление" : "без сокета"}
              </span>
            </div>
          </div>

          <div className="flex items-center gap-2">
            <button
              type="button"
              className="btn-primary"
              disabled={!canStartSearch || startMutation.isPending}
              title={
                canStartSearch
                  ? "Запустить поиск по всем позициям"
                  : "Поиск запускается после проверки требований"
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
      </div>

      {tender.error_message ? <p className="banner-error">{tender.error_message}</p> : null}
      {actionError ? <p className="banner-warn">{actionError}</p> : null}

      {run ? (
        <div className="panel">
          <div className="panel-body space-y-3">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <span className="section-title">Текущий запуск</span>
              <StatusBadge status={run.status} />
            </div>
            <div className="progress">
              <div className="progress-bar" style={{ width: `${progress}%` }} />
            </div>
            <dl className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-6">
              <div className="kv">
                <dt>Позиции</dt>
                <dd>
                  {run.components_done}/{run.components_total}
                  {run.components_failed > 0 ? (
                    <span className="ml-1 text-red-700">· {run.components_failed} с ошибкой</span>
                  ) : null}
                </dd>
              </div>
              <div className="kv">
                <dt>Запросов</dt>
                <dd>{run.queries_issued}</dd>
              </div>
              <div className="kv">
                <dt>Найдено</dt>
                <dd>{run.results_found}</dd>
              </div>
              <div className="kv">
                <dt>Страниц</dt>
                <dd>{run.pages_fetched}</dd>
              </div>
              <div className="kv">
                <dt>Начат</dt>
                <dd>{formatDateTime(run.started_at)}</dd>
              </div>
              <div className="kv">
                <dt>Завершён</dt>
                <dd>{formatDateTime(run.finished_at)}</dd>
              </div>
            </dl>
          </div>
        </div>
      ) : null}

      {feed.length > 0 ? (
        <div className="feed panel px-4 py-3">
          {feed.map((message, index) => (
            <p key={`${message.event_type ?? message.type}-${index}`}>
              <span className="text-zinc-400">{message.stage ?? message.type}</span>{" "}
              {message.message ?? ""}
            </p>
          ))}
        </div>
      ) : null}

      <div className="tabs">
        {TABS.map((item) => (
          <button
            key={item.key}
            type="button"
            onClick={() => setTab(item.key)}
            className={`tab ${tab === item.key ? "tab-active" : ""}`}
          >
            {item.label}
            {counts[item.key] !== null ? (
              <span className="tab-count">{counts[item.key]}</span>
            ) : null}
          </button>
        ))}
      </div>

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
