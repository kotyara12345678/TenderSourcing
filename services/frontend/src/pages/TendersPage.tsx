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

  return (
    <div className="space-y-6">
      <section className="card p-6">
        <h2 className="text-base font-semibold text-slate-900">Новый тендер</h2>
        <p className="mt-1 text-sm text-slate-500">
          Загрузите документацию — позиции и требования извлекаются автоматически.
        </p>
        <form className="mt-4 grid gap-3 md:grid-cols-[2fr_3fr_auto]" onSubmit={onSubmit}>
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
        </form>
        {formError ? <p className="mt-2 text-sm text-red-600">{formError}</p> : null}
      </section>

      <section className="card">
        <div className="flex flex-wrap items-center gap-3 border-b border-slate-200 p-4">
          <input
            className="input max-w-xs"
            placeholder="Поиск по названию"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
          <select
            className="input max-w-[220px]"
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
          <span className="ml-auto text-sm text-slate-500">
            Всего: {tendersQuery.data?.total ?? 0}
          </span>
        </div>

        {tendersQuery.isPending ? (
          <p className="p-6 text-sm text-slate-500">Загрузка…</p>
        ) : tendersQuery.isError ? (
          <p className="p-6 text-sm text-red-600">{tendersQuery.error.message}</p>
        ) : items.length === 0 ? (
          <p className="p-6 text-sm text-slate-500">Тендеров пока нет.</p>
        ) : (
          <table className="table">
            <thead>
              <tr>
                <th>Тендер</th>
                <th>Статус</th>
                <th>Позиций</th>
                <th>Создан</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {items.map((tender) => (
                <tr key={tender.id}>
                  <td>
                    <Link className="font-medium" to={`/tenders/${tender.id}`}>
                      {tender.name}
                    </Link>
                    {tender.description ? (
                      <p className="mt-0.5 line-clamp-1 text-xs text-slate-500">{tender.description}</p>
                    ) : null}
                  </td>
                  <td>
                    <StatusBadge status={tender.status} />
                  </td>
                  <td>{tender.components_count}</td>
                  <td className="whitespace-nowrap text-slate-500">
                    {formatDateTime(tender.created_at)}
                  </td>
                  <td className="text-right">
                    <Link className="btn-secondary" to={`/tenders/${tender.id}`}>
                      Открыть
                    </Link>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </div>
  );
}
