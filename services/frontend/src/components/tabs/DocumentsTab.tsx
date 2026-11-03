import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useRef, useState } from "react";

import { deleteDocument, listDocuments, uploadDocument } from "@/api/endpoints";
import { StatusBadge } from "@/components/StatusBadge";
import { formatBytes, formatDateTime } from "@/lib/format";

export default function DocumentsTab({ tenderId }: { tenderId: string }): JSX.Element {
  const queryClient = useQueryClient();
  const fileInput = useRef<HTMLInputElement | null>(null);
  const [error, setError] = useState<string | null>(null);

  const documentsQuery = useQuery({
    queryKey: ["documents", tenderId],
    queryFn: () => listDocuments(tenderId),
    refetchInterval: 5_000,
  });

  const uploadMutation = useMutation({
    mutationFn: (file: File) => uploadDocument(tenderId, file),
    onSuccess: () => {
      setError(null);
      if (fileInput.current) fileInput.current.value = "";
      void queryClient.invalidateQueries({ queryKey: ["documents", tenderId] });
      void queryClient.invalidateQueries({ queryKey: ["tender", tenderId] });
    },
    onError: (uploadError: Error) => setError(uploadError.message),
  });

  const deleteMutation = useMutation({
    mutationFn: (documentId: string) => deleteDocument(documentId),
    onSuccess: () => {
      setError(null);
      void queryClient.invalidateQueries({ queryKey: ["documents", tenderId] });
    },
    onError: (deleteError: Error) => setError(deleteError.message),
  });

  const documents = documentsQuery.data ?? [];

  return (
    <div className="space-y-4">
      <div className="card flex flex-wrap items-center gap-3 p-4">
        <input
          ref={fileInput}
          type="file"
          accept=".pdf,.docx,.xlsx,.txt,.rtf,.odt"
          className="text-sm"
          onChange={(event) => {
            const file = event.target.files?.[0];
            if (file) {
              uploadMutation.mutate(file);
            }
          }}
        />
        <span className="text-xs text-slate-500">
          PDF, DOCX, XLSX, TXT, RTF, ODT · до 25 МБ · дубликаты по SHA-256 не загружаются повторно
        </span>
        {uploadMutation.isPending ? (
          <span className="text-xs text-brand-600">Загрузка…</span>
        ) : null}
      </div>

      {error ? (
        <p className="rounded-lg bg-red-50 px-3 py-2 text-sm text-red-700">{error}</p>
      ) : null}

      <div className="card">
        {documentsQuery.isPending ? (
          <p className="p-6 text-sm text-slate-500">Загрузка…</p>
        ) : documents.length === 0 ? (
          <p className="p-6 text-sm text-slate-500">Документы ещё не загружены.</p>
        ) : (
          <table className="table">
            <thead>
              <tr>
                <th>Файл</th>
                <th>Размер</th>
                <th>Разбор</th>
                <th>Требования</th>
                <th>Страниц</th>
                <th>Загружен</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {documents.map((document) => (
                <tr key={document.id}>
                  <td>
                    <p className="font-medium">{document.original_filename}</p>
                    {document.error_message ? (
                      <p className="mt-0.5 text-xs text-red-600">{document.error_message}</p>
                    ) : null}
                  </td>
                  <td className="whitespace-nowrap">{formatBytes(document.size_bytes)}</td>
                  <td>
                    <StatusBadge status={document.status} />
                  </td>
                  <td>
                    <StatusBadge status={document.ai_status} />
                  </td>
                  <td>{document.page_count ?? "—"}{document.ocr_used ? " (OCR)" : ""}</td>
                  <td className="whitespace-nowrap text-slate-500">
                    {formatDateTime(document.uploaded_at)}
                  </td>
                  <td className="text-right">
                    <button
                      type="button"
                      className="btn-danger"
                      disabled={deleteMutation.isPending}
                      onClick={() => deleteMutation.mutate(document.id)}
                    >
                      Удалить
                    </button>
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
