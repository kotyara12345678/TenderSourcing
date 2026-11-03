import type { TenderStatus } from "@/api/types";

const LABELS: Record<string, string> = {
  CREATED: "Создан",
  UPLOADING: "Загрузка файлов",
  DOCUMENT_PROCESSING: "Разбор документов",
  REQUIREMENT_EXTRACTION: "Извлечение требований",
  REQUIREMENTS_REVIEW: "Проверка требований",
  SEARCH_QUEUED: "Поиск в очереди",
  SEARCHING: "Поиск",
  FETCHING: "Загрузка страниц",
  PRODUCT_EXTRACTION: "Извлечение товаров",
  MATCHING: "Сопоставление",
  RANKING: "Ранжирование",
  REPORT_GENERATION: "Формирование отчёта",
  COMPLETED: "Завершён",
  COMPLETED_WITH_ERRORS: "Завершён с ошибками",
  FAILED: "Ошибка",
  CANCELLED: "Отменён",
  // document / task statuses
  UPLOADED: "Загружен",
  QUEUED: "В очереди",
  PROCESSING: "Обработка",
  EXTRACTED: "Извлечён",
  DONE: "Готово",
  RUNNING: "Выполняется",
  PENDING: "Ожидает",
  READY: "Готов",
  SKIPPED: "Пропущен",
  // match statuses
  FULL: "Полное соответствие",
  TOLERANCE: "В пределах допуска",
  PARTIAL: "Частичное",
  MISMATCH: "Не соответствует",
  INSUFFICIENT: "Недостаточно данных",
  match: "Совпадает",
  tolerance: "Допуск",
  partial: "Частично",
  mismatch: "Не совпадает",
  missing: "Нет данных",
};

const TONES: Record<string, string> = {
  Completed: "bg-emerald-50 text-emerald-700 border-emerald-200",
  Running: "bg-brand-50 text-brand-700 border-brand-200",
  Failed: "bg-red-50 text-red-700 border-red-200",
  Neutral: "bg-slate-100 text-slate-700 border-slate-200",
};

const RUNNING = new Set([
  "UPLOADING",
  "DOCUMENT_PROCESSING",
  "REQUIREMENT_EXTRACTION",
  "SEARCH_QUEUED",
  "SEARCHING",
  "FETCHING",
  "PRODUCT_EXTRACTION",
  "MATCHING",
  "RANKING",
  "REPORT_GENERATION",
  "PROCESSING",
  "RUNNING",
  "QUEUED",
  "PENDING",
]);

const COMPLETED = new Set([
  "COMPLETED",
  "COMPLETED_WITH_ERRORS",
  "EXTRACTED",
  "DONE",
  "READY",
  "FULL",
  "TOLERANCE",
  "PARTIAL",
  "match",
  "tolerance",
  "partial",
]);

const FAILED = new Set(["FAILED", "MISMATCH", "INSUFFICIENT", "CANCELLED", "mismatch", "missing"]);

function tone(status: string): string {
  if (FAILED.has(status)) return TONES.Failed ?? "";
  if (RUNNING.has(status)) return TONES.Running ?? "";
  if (COMPLETED.has(status)) return TONES.Completed ?? "";
  return TONES.Neutral ?? "";
}

export function StatusBadge({ status }: { status: string }): JSX.Element {
  const label = LABELS[status] ?? status;
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs font-medium ${tone(
        status,
      )}`}
    >
      {label}
    </span>
  );
}

export function isTerminalStatus(status: TenderStatus): boolean {
  return ["COMPLETED", "COMPLETED_WITH_ERRORS", "FAILED", "CANCELLED"].includes(status);
}
