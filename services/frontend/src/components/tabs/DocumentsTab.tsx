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
      <div className="panel">
        <div className="panel-body flex flex-wrap items-center gap-3">
          <input
            ref={fileInput}
            type="file"
            accept=".pdf,.docx,.xlsx,.txt,.rtf,.odt"
            className="text-[12px] text-zinc-600 file:mr-3 file:cursor-pointer file:rounded-md
              file:border file:border-zinc-300 file:bg-white file:px-2.5 file:py-1
              file:text-[12px] file:font-medium file:text-zinc-700 hover:file:border-zinc-400"
            onChange={(event) => {
              const file = event.target.files?.[0];
              if (file) {
                uploadMutation.mutate(file);
              }
            }}
          />
          <span className="field-hint">
            PDF, DOCX, XLSX, TXT, RTF, ODT · до 25 МБ · дубликаты по SHA-256 пропускаются
          </span>
          {uploadMutation.isPending ? (
            <span className="ml-auto text-[12px] text-zinc-500">Загрузка…</span>
          ) : null}
        </div>
      </div>

      {error ? <p className="banner-error">{error}</p> : null}

      <div className="panel">
        {documentsQuery.isPending ? (
          <p className="empty">Загрузка…</p>
        ) : documents.length === 0 ? (
          <p className="empty">Документы ещё не загружены.</p>
        ) : (
          <table className="table">
            <thead>
              <tr>
                <th>Файл</th>
                <th className="w-24 text-right">Размер</th>
                <th className="w-32">Разбор</th>
                <th className="w-36">Требования</th>
                <th className="w-28">Страниц</th>
                <th className="w-40">Загружен</th>
                <th className="w-20" />
              </tr>
            </thead>
            <tbody>
              {documents.map((document) => (
                <tr key={document.id}>
                  <td>
                    <p className="font-medium">{document.original_filename}</p>
                    {document.error_message ? (
                      <p className="mt-0.5 text-[12px] text-red-700">{document.error_message}</p>
                    ) : null}
                  </td>
                  <td className="num whitespace-nowrap text-right">
                    {formatBytes(document.size_bytes)}
                  </td>
                  <td>
                    <StatusBadge status={document.status} />
                  </td>
                  <td>
                    <StatusBadge status={document.ai_status} />
                  </td>
                  <td className="num text-zinc-600">
                    {document.page_count ?? "—"}
                    {document.ocr_used ? <span className="ml-1 text-zinc-400">OCR</span> : null}
                  </td>
                  <td className="whitespace-nowrap text-[12px] text-zinc-500">
                    {formatDateTime(document.uploaded_at)}
                  </td>
                  <td className="text-right">
                    <button
                      type="button"
                      className="btn-danger btn-sm"
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
