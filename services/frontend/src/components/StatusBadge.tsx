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
  COMPLETED_WITH_ERRORS: "Завершён с замечаниями",
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
  MATCHED: "Сопоставлено",
  SEARCHED: "Найдено",
  SKIPPED: "Пропущен",
  REVIEWED: "Проверено",
  DRAFT: "Черновик",
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

const BUSY = new Set([
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

const ERROR = new Set(["FAILED", "MISMATCH", "CANCELLED", "mismatch"]);

const WARN = new Set(["INSUFFICIENT", "missing", "COMPLETED_WITH_ERRORS"]);

const DONE = new Set([
  "COMPLETED",
  "COMPLETED_WITH_ERRORS",
  "EXTRACTED",
  "DONE",
  "READY",
  "FULL",
  "match",
  "MATCHED",
]);

function dotClass(status: string): string {
  if (ERROR.has(status)) return "status-dot status-dot-error";
  if (WARN.has(status)) return "status-dot status-dot-warn";
  if (BUSY.has(status)) return "status-dot status-dot-busy animate-pulse";
  if (DONE.has(status)) return "status-dot status-dot-live";
  return "status-dot";
}

export function StatusBadge({ status }: { status: string }): JSX.Element {
  const label = LABELS[status] ?? status;
  return (
    <span className="status" title={status}>
      <span className={dotClass(status)} aria-hidden="true" />
      {label}
    </span>
  );
}

export function isTerminalStatus(status: TenderStatus): boolean {
  return ["COMPLETED", "COMPLETED_WITH_ERRORS", "FAILED", "CANCELLED"].includes(status);
}
