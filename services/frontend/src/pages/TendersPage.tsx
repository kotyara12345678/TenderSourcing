import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import type { FormEvent } from "react";
import { Link } from "react-router-dom";

import { createTender, listTenders } from "@/api/endpoints";
import type { TenderStatus } from "@/api/types";
import { StatusBadge } from "@/components/StatusBadge";
import { formatDateTime } from "@/lib/format";

const STATUSES: TenderStatus[] = [
  "CREATED",
  "DOCUMENT_PROCESSING",
  "REQUIREMENT_EXTRACTION",
  "REQUIREMENTS_REVIEW",
  "SEARCH_QUEUED",
  "SEARCHING",
  "FETCHING",
  "PRODUCT_EXTRACTION",
  "MATCHING",
  "RANKING",
  "REPORT_GENERATION",
  "COMPLETED",
  "COMPLETED_WITH_ERRORS",
  "FAILED",
  "CANCELLED",
];

export default function TendersPage(): JSX.Element {
  const queryClient = useQueryClient();
  const [status, setStatus] = useState("");
  const [query, setQuery] = useState("");
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [formError, setFormError] = useState<string | null>(null);

  const tendersQuery = useQuery({
    queryKey: ["tenders", status, query],
    queryFn: () => listTenders({ status: status || undefined, q: query || undefined, pageSize: 50 }),
  });

  const createMutation = useMutation({
    mutationFn: () => createTender({ name: name.trim(), description: description.trim() }),
    onSuccess: () => {
      setName("");
      setDescription("");
      setFormError(null);
      void queryClient.invalidateQueries({ queryKey: ["tenders"] });
    },
    onError: (error: Error) => {
      setFormError(error.message);
    },
  });

  const onSubmit = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    if (!name.trim()) {
      setFormError("Укажите название тендера");
      return;
    }
    createMutation.mutate();
  };

  const items = tendersQuery.data?.items ?? [];
  const total = tendersQuery.data?.total ?? 0;

  return (
    <div className="space-y-7">
      <div className="page-head">
        <div>
          <h1 className="page-title">Тендеры</h1>
          <p className="page-subtitle">
            Загрузите документацию — позиции и требования извлекаются автоматически.
          </p>
        </div>
        <span className="text-[12px] tabular-nums text-zinc-400">всего: {total}</span>
      </div>

      <form className="panel" onSubmit={onSubmit}>
        <div className="panel-body grid gap-2 sm:grid-cols-[1fr_1.4fr_auto]">
          <input
            className="input"
            placeholder="Название тендера"
            value={name}
            maxLength={200}
            onChange={(event) => setName(event.target.value)}
          />
          <input
            className="input"
            placeholder="Описание (необязательно)"
            value={description}
            maxLength={4000}
            onChange={(event) => setDescription(event.target.value)}
          />
          <button type="submit" className="btn-primary" disabled={createMutation.isPending}>
            {createMutation.isPending ? "Создание…" : "Создать"}
          </button>
        </div>
        {formError ? (
          <p className="border-t border-zinc-200 px-4 py-2 text-[12px] text-red-700">{formError}</p>
        ) : null}
      </form>

      <div className="panel">
        <div className="panel-head">
          <div className="flex flex-wrap items-center gap-2">
            <input
              className="input input-sm w-56"
              placeholder="Поиск по названию"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
            <select
              className="select input-sm w-48"
              value={status}
              onChange={(event) => setStatus(event.target.value)}
            >
              <option value="">Все статусы</option>
              {STATUSES.map((value) => (
                <option key={value} value={value}>
                  {value}
                </option>
              ))}
            </select>
            {status || query ? (
              <button
                type="button"
                className="btn-ghost btn-sm"
                onClick={() => {
                  setStatus("");
                  setQuery("");
                }}
              >
                Сбросить
              </button>
            ) : null}
          </div>
        </div>

        {tendersQuery.isPending ? (
          <p className="empty">Загрузка…</p>
        ) : tendersQuery.isError ? (
          <p className="empty text-red-700">{tendersQuery.error.message}</p>
        ) : items.length === 0 ? (
          <p className="empty">Тендеров пока нет — создайте первый выше.</p>
        ) : (
          <table className="table">
            <thead>
              <tr>
                <th className="w-8">№</th>
                <th>Тендер</th>
                <th className="w-44">Статус</th>
                <th className="w-20 text-right">Позиций</th>
                <th className="w-40">Создан</th>
              </tr>
            </thead>
            <tbody>
              {items.map((tender, index) => (
                <tr key={tender.id} className="group">
                  <td className="text-[12px] tabular-nums text-zinc-400">{index + 1}</td>
                  <td>
                    <Link className="font-medium no-underline hover:underline" to={`/tenders/${tender.id}`}>
                      {tender.name}
                    </Link>
                    {tender.description ? (
                      <p className="mt-0.5 line-clamp-1 text-[12px] text-zinc-500">
                        {tender.description}
                      </p>
                    ) : null}
                  </td>
                  <td>
                    <StatusBadge status={tender.status} />
                  </td>
                  <td className="num text-right">{tender.components_count}</td>
                  <td className="whitespace-nowrap text-[12px] text-zinc-500">
                    {formatDateTime(tender.created_at)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}
