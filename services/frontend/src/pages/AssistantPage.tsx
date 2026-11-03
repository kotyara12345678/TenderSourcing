import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";

import { ApiError } from "@/api/client";
import { applyIntake, getIntake, parseIntake } from "@/api/endpoints";
import type {
  ApplyPosition,
  IntakeResult,
  IntakeStep,
  IntakeView,
} from "@/api/types";
import { formatNumber } from "@/lib/format";

/** One turn of the conversation: the operator's words plus what came back. */
interface Turn {
  id: string;
  text: string;
  intakeId: string | null;
  intake: IntakeView | null;
  draft: Draft | null;
  error: string | null;
  applying: boolean;
  startedAt: number;
}

interface Draft {
  name: string;
  positions: ApplyPosition[];
}

const EXAMPLES = [
  "200 м кабеля ВВГнг-LS 3х2,5 на 660 В и 10 автоматов ВА47-29 на 16 А",
  "50 светильников LED 36 Вт IP65 и 300 м гофры 20 мм",
  "5 кг электродов УОНИ-13/55 3 мм, 2 упаковки стяжек 200 мм",
];

const PENDING: ReadonlyArray<IntakeView["status"]> = ["QUEUED", "RUNNING"];

function draftFrom(result: IntakeResult): Draft {
  return {
    name: result.tender_name || "Закупка из AI-запроса",
    positions: result.positions.map((position) => ({
      name: position.name,
      quantity: position.quantity,
      unit: position.unit,
      category: position.category,
      requirements: position.requirements.map((requirement) => ({
        ...requirement,
      })),
    })),
  };
}

function newId(): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `t-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

export default function AssistantPage(): JSX.Element {
  const navigate = useNavigate();
  const [text, setText] = useState("");
  const [turns, setTurns] = useState<Turn[]>([]);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const bottomRef = useRef<HTMLDivElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  // A turn is pending from the moment the request is queued: the intake view
  // itself only exists after the first poll response.
  const pendingIds = useMemo(
    () =>
      turns
        .filter(
          (turn) =>
            turn.intakeId &&
            (!turn.intake || PENDING.includes(turn.intake.status)),
        )
        .map((turn) => turn.intakeId)
        .filter((id): id is string => Boolean(id)),
    [turns],
  );
  const pendingKey = pendingIds.join(",");

  useEffect(() => {
    if (!pendingKey) return;
    const ids = pendingKey.split(",");
    let cancelled = false;

    const tick = async (): Promise<void> => {
      const views = await Promise.all(
        ids.map((id) => getIntake(id).catch(() => null)),
      );
      if (cancelled) return;
      setTurns((prev) =>
        prev.map((turn) => {
          const view = views.find(
            (candidate) => candidate?.id === turn.intakeId,
          );
          if (!view) return turn;
          const draft =
            turn.draft ??
            (view.status === "COMPLETED" && view.result
              ? draftFrom(view.result)
              : null);
          return { ...turn, intake: view, draft };
        }),
      );
    };

    void tick();
    const timer = window.setInterval(() => {
      void tick();
    }, 700);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [pendingKey]);

  const draftCount = turns.filter((turn) => turn.draft).length;

  useEffect(() => {
    // The result card renders a tick after the status flips to COMPLETED, so the
    // scroll waits for the next frame instead of stopping above the card.
    const timer = window.setTimeout(() => {
      // Scroll the document, not the anchor: the last card must clear the fixed
      // composer instead of hiding behind it.
      window.scrollTo({
        top: document.documentElement.scrollHeight,
        behavior: "smooth",
      });
    }, 120);
    return () => window.clearTimeout(timer);
  }, [turns.length, pendingKey, draftCount]);

  const submit = async (): Promise<void> => {
    const value = text.trim();
    if (value.length < 8 || submitting) return;

    const id = newId();
    const turn: Turn = {
      id,
      text: value,
      intakeId: null,
      intake: null,
      draft: null,
      error: null,
      applying: false,
      startedAt: Date.now(),
    };
    setTurns((prev) => [...prev, turn]);
    setText("");
    setSubmitting(true);
    setError(null);

    try {
      const { id: intakeId } = await parseIntake(value);
      setTurns((prev) =>
        prev.map((item) => (item.id === id ? { ...item, intakeId } : item)),
      );
    } catch (cause) {
      const message =
        cause instanceof ApiError
          ? cause.message
          : "Не удалось отправить запрос";
      setTurns((prev) =>
        prev.map((item) =>
          item.id === id ? { ...item, error: message } : item,
        ),
      );
      setError(message);
    } finally {
      setSubmitting(false);
      textareaRef.current?.focus();
    }
  };

  const updateDraft = (
    turnId: string,
    updater: (draft: Draft) => Draft,
  ): void => {
    setTurns((prev) =>
      prev.map((turn) =>
        turn.id === turnId && turn.draft
          ? { ...turn, draft: updater(turn.draft) }
          : turn,
      ),
    );
  };

  const createTender = async (turn: Turn): Promise<void> => {
    if (!turn.intakeId || !turn.draft) return;
    const draft = turn.draft;
    if (!draft.name.trim() || draft.positions.length === 0) {
      setTurns((prev) =>
        prev.map((item) =>
          item.id === turn.id
            ? { ...item, error: "Нужны название и хотя бы одна позиция" }
            : item,
        ),
      );
      return;
    }
    setTurns((prev) =>
      prev.map((item) =>
        item.id === turn.id ? { ...item, applying: true, error: null } : item,
      ),
    );
    try {
      const result = await applyIntake(turn.intakeId, {
        name: draft.name.trim(),
        description: "",
        positions: draft.positions.map((position) => ({
          ...position,
          quantity: Number.isFinite(position.quantity) ? position.quantity : 1,
        })),
      });
      navigate(`/tenders/${result.tender_id}`);
    } catch (cause) {
      const message =
        cause instanceof ApiError ? cause.message : "Не удалось создать тендер";
      setTurns((prev) =>
        prev.map((item) =>
          item.id === turn.id
            ? { ...item, applying: false, error: message }
            : item,
        ),
      );
    }
  };

  return (
    <div className="mx-auto w-full max-w-[820px] pb-64">
      <div className="page-head">
        <div>
          <h1 className="page-title">AI-запрос</h1>
          <p className="page-subtitle">
            Опишите закупку обычными словами — система соберёт позиции,
            количества и характеристики.
          </p>
        </div>
      </div>

      {turns.length === 0 ? (
        <div className="mb-6 space-y-3">
          <div className="panel panel-body">
            <p className="text-[13px] leading-relaxed text-zinc-600">
              Пишите так, как объяснили бы снабженцу: «нужно 200 метров кабеля
              ВВГнг 3×2,5 на 660 вольт и десяток автоматов на 16 ампер». Из
              запроса соберутся позиции, единицы и требования — их можно
              поправить перед созданием тендера.
            </p>
          </div>
          <div className="flex flex-wrap gap-2">
            {EXAMPLES.map((example) => (
              <button
                key={example}
                type="button"
                className="suggest"
                onClick={() => {
                  setText(example);
                  textareaRef.current?.focus();
                }}
              >
                {example}
              </button>
            ))}
          </div>
        </div>
      ) : null}

      <div className="space-y-6">
        {turns.map((turn) => (
          <TurnView
            key={turn.id}
            turn={turn}
            onUpdateDraft={updateDraft}
            onCreateTender={createTender}
          />
        ))}
        <div ref={bottomRef} />
      </div>

      <div className="fixed inset-x-0 bottom-0 z-20 border-t border-zinc-200 bg-white/85 backdrop-blur">
        <div className="mx-auto w-full max-w-[820px] px-5 py-3">
          <div className="composer">
            <textarea
              ref={textareaRef}
              value={text}
              rows={3}
              placeholder="Что нужно закупить?"
              onChange={(event) => setText(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !event.shiftKey) {
                  event.preventDefault();
                  void submit();
                }
              }}
            />
            <div className="composer-bar">
              <span className="text-[11.5px] text-zinc-400">
                Enter — отправить, Shift+Enter — новая строка
              </span>
              <button
                type="button"
                className="btn-primary"
                disabled={submitting || text.trim().length < 8}
                onClick={() => {
                  void submit();
                }}
              >
                {submitting ? "Отправляю…" : "Разобрать запрос"}
              </button>
            </div>
          </div>
          {error ? <div className="banner-error mt-2">{error}</div> : null}
        </div>
      </div>
    </div>
  );
}

function TurnView({
  turn,
  onUpdateDraft,
  onCreateTender,
}: {
  turn: Turn;
  onUpdateDraft: (turnId: string, updater: (draft: Draft) => Draft) => void;
  onCreateTender: (turn: Turn) => Promise<void>;
}): JSX.Element {
  return (
    <div className="space-y-3">
      <div className="bubble-user">{turn.text}</div>
      {turn.intake ? (
        <TraceCard turn={turn} />
      ) : (
        <div className="trace trace-head">
          <span className="step-spinner" />
          <span className="step-title text-[12.5px]">Отправляю запрос…</span>
        </div>
      )}

      {turn.error ? <div className="banner-error">{turn.error}</div> : null}

      {turn.intake?.status === "FAILED" ? (
        <div className="banner-error">
          {turn.intake.error_message ?? "Не удалось разобрать запрос"}
        </div>
      ) : null}

      {turn.draft && turn.intake?.status === "COMPLETED" ? (
        <DraftCard
          turn={turn}
          draft={turn.draft}
          onUpdateDraft={onUpdateDraft}
          onCreateTender={onCreateTender}
        />
      ) : null}
    </div>
  );
}

function TraceCard({ turn }: { turn: Turn }): JSX.Element {
  const intake = turn.intake;
  const steps = intake?.steps ?? [];
  const done = steps.filter((step) => step.status === "done").length;
  const live = intake ? PENDING.includes(intake.status) : false;
  const failed = intake?.status === "FAILED";

  return (
    <div className="trace">
      <div className="trace-head">
        {live ? (
          <span className="step-spinner" />
        ) : (
          <span
            className={`step-icon ${failed ? "step-icon-error" : "step-icon-done"}`}
          >
            {failed ? <CrossIcon /> : <CheckIcon />}
          </span>
        )}
        <span className="text-[12.5px] font-medium text-zinc-900">
          {live
            ? "Разбираю запрос"
            : failed
              ? "Запрос не разобран"
              : "Запрос разобран"}
        </span>
        <span className="ml-auto flex items-center gap-3 text-[11.5px] text-zinc-400">
          <Elapsed startedAt={turn.startedAt} running={live} />
          <span className="tabular-nums">
            {done}/{steps.length || 4}
          </span>
        </span>
      </div>
      <div className="px-3.5 pb-2">
        <div className="meter">
          <div
            className={`meter-fill ${live ? "meter-fill-live" : ""}`}
            style={{
              width: `${Math.round(
                Math.max(live ? 0.06 : 0, Math.min(1, intake?.progress ?? 0)) *
                  100,
              )}%`,
            }}
          />
        </div>
      </div>
      {steps.length > 0 ? (
        <div className="trace-list">
          {steps.map((step) => (
            <StepRow key={step.key} step={step} />
          ))}
        </div>
      ) : null}
    </div>
  );
}

function StepRow({ step }: { step: IntakeStep }): JSX.Element {
  return (
    <div className={`step ${step.status === "running" ? "step-live" : ""}`}>
      {step.status === "running" ? (
        <span className="step-spinner" />
      ) : (
        <span
          className={`step-icon ${
            step.status === "done"
              ? "step-icon-done"
              : step.status === "error"
                ? "step-icon-error"
                : "step-icon-idle"
          }`}
        >
          {step.status === "done" ? (
            <CheckIcon />
          ) : step.status === "error" ? (
            <CrossIcon />
          ) : (
            <span className="h-[3px] w-[3px] rounded-full bg-zinc-300" />
          )}
        </span>
      )}
      <span className="min-w-0">
        <span className="step-title">{step.title}</span>
        {step.detail ? (
          <span className="step-detail"> — {step.detail}</span>
        ) : null}
      </span>
    </div>
  );
}

function DraftCard({
  turn,
  draft,
  onUpdateDraft,
  onCreateTender,
}: {
  turn: Turn;
  draft: Draft;
  onUpdateDraft: (turnId: string, updater: (draft: Draft) => Draft) => void;
  onCreateTender: (turn: Turn) => Promise<void>;
}): JSX.Element {
  const summary = turn.intake?.result?.summary;
  const requirementCount = draft.positions.reduce(
    (total, position) => total + position.requirements.length,
    0,
  );

  const patchPosition = (
    index: number,
    patch: Partial<ApplyPosition>,
  ): void => {
    onUpdateDraft(turn.id, (current) => ({
      ...current,
      positions: current.positions.map((position, positionIndex) =>
        positionIndex === index ? { ...position, ...patch } : position,
      ),
    }));
  };

  return (
    <div className="panel">
      <div className="panel-head">
        <div className="min-w-0">
          <div className="section-title">Позиции из запроса</div>
          {summary ? (
            <div className="mt-0.5 truncate text-[12px] text-zinc-500">
              {summary}
            </div>
          ) : null}
        </div>
        <div className="text-[11.5px] tabular-nums text-zinc-400">
          {draft.positions.length} поз. · {requirementCount} хар.
        </div>
      </div>

      <div className="panel-body space-y-4">
        <div>
          <label className="label" htmlFor={`name-${turn.id}`}>
            Название тендера
          </label>
          <input
            id={`name-${turn.id}`}
            className="input"
            value={draft.name}
            onChange={(event) =>
              onUpdateDraft(turn.id, (current) => ({
                ...current,
                name: event.target.value,
              }))
            }
          />
        </div>

        <div className="space-y-3">
          {draft.positions.map((position, index) => (
            <div
              key={`${turn.id}-${index}`}
              className="rounded-md border border-zinc-200 p-3"
            >
              <div className="flex flex-wrap items-start gap-2">
                <span className="mt-2 w-4 shrink-0 text-[12px] tabular-nums text-zinc-400">
                  {index + 1}
                </span>
                <input
                  className="input input-sm min-w-0 flex-1"
                  aria-label={`Позиция ${index + 1}`}
                  value={position.name}
                  onChange={(event) =>
                    patchPosition(index, { name: event.target.value })
                  }
                />
                <input
                  className="input input-sm w-20 text-right tabular-nums"
                  type="number"
                  min={0}
                  step="0.001"
                  aria-label={`Количество позиции ${index + 1}`}
                  value={
                    Number.isFinite(position.quantity) ? position.quantity : 1
                  }
                  onChange={(event) => {
                    const next = Number(event.target.value);
                    patchPosition(index, {
                      quantity: Number.isFinite(next) ? next : 1,
                    });
                  }}
                />
                <input
                  className="input input-sm w-16"
                  aria-label={`Единица позиции ${index + 1}`}
                  value={position.unit}
                  onChange={(event) =>
                    patchPosition(index, { unit: event.target.value })
                  }
                />
                <button
                  type="button"
                  className="btn-ghost btn-sm"
                  onClick={() =>
                    onUpdateDraft(turn.id, (current) => ({
                      ...current,
                      positions: current.positions.filter(
                        (_, itemIndex) => itemIndex !== index,
                      ),
                    }))
                  }
                >
                  Убрать
                </button>
              </div>

              {position.requirements.length > 0 ? (
                <div className="mt-2 flex flex-wrap gap-1.5 pl-6">
                  {position.requirements.map(
                    (requirement, requirementIndex) => (
                      <span
                        className="chip"
                        key={`${requirement.name}-${requirementIndex}`}
                      >
                        <span>{requirement.name}</span>
                        <span className="chip-value">
                          {requirement.value_text ||
                            formatNumber(requirement.value_num)}
                        </span>
                        {requirement.unit ? (
                          <span className="text-zinc-400">
                            {requirement.unit}
                          </span>
                        ) : null}
                        <button
                          type="button"
                          className="chip-remove"
                          aria-label={`Удалить характеристику ${requirement.name}`}
                          onClick={() =>
                            patchPosition(index, {
                              requirements: position.requirements.filter(
                                (_, itemIndex) =>
                                  itemIndex !== requirementIndex,
                              ),
                            })
                          }
                        >
                          <CrossIcon />
                        </button>
                      </span>
                    ),
                  )}
                </div>
              ) : null}
            </div>
          ))}
        </div>

        {draft.positions.length === 0 ? (
          <p className="text-[12.5px] text-zinc-500">
            Все позиции убраны — верните хотя бы одну, чтобы создать тендер.
          </p>
        ) : null}
      </div>

      <div className="divider" />
      <div className="flex flex-wrap items-center justify-between gap-2 px-4 py-3">
        <span className="text-[11.5px] text-zinc-400">
          Тендер создастся в статусе подготовки — поиск запускается на его
          странице.
        </span>
        <button
          type="button"
          className="btn-primary"
          disabled={turn.applying || draft.positions.length === 0}
          onClick={() => {
            void onCreateTender(turn);
          }}
        >
          {turn.applying ? "Создаю…" : "Создать тендер"}
        </button>
      </div>
    </div>
  );
}

function Elapsed({
  startedAt,
  running,
}: {
  startedAt: number;
  running: boolean;
}): JSX.Element | null {
  const [seconds, setSeconds] = useState(0);

  useEffect(() => {
    if (!running) return;
    const tick = (): void => {
      setSeconds(Math.floor((Date.now() - startedAt) / 1000));
    };
    tick();
    const timer = window.setInterval(tick, 1000);
    return () => window.clearInterval(timer);
  }, [running, startedAt]);

  if (!running) return null;
  return (
    <span className="tabular-nums">
      {String(Math.floor(seconds / 60)).padStart(2, "0")}:
      {String(seconds % 60).padStart(2, "0")}
    </span>
  );
}

function CheckIcon(): JSX.Element {
  return (
    <svg
      viewBox="0 0 24 24"
      className="h-2.5 w-2.5"
      fill="none"
      stroke="currentColor"
      strokeWidth="3.5"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <polyline points="20 6 9 17 4 12" />
    </svg>
  );
}

function CrossIcon(): JSX.Element {
  return (
    <svg
      viewBox="0 0 24 24"
      className="h-2.5 w-2.5"
      fill="none"
      stroke="currentColor"
      strokeWidth="3.5"
      strokeLinecap="round"
    >
      <line x1="18" y1="6" x2="6" y2="18" />
      <line x1="6" y1="6" x2="18" y2="18" />
    </svg>
  );
}
