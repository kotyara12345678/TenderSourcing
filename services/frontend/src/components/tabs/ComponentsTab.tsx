import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import type { FormEvent } from "react";

import {
  createComponent,
  createRequirement,
  deleteComponent,
  deleteRequirement,
  listComponents,
  listRequirements,
} from "@/api/endpoints";
import type { RequirementInput } from "@/api/types";
import { StatusBadge } from "@/components/StatusBadge";

const DATA_TYPES = [
  "STRING",
  "NUMBER",
  "RANGE",
  "BOOLEAN",
  "ENUM",
  "STANDARD",
  "MATERIAL",
  "MODEL",
  "DIMENSION",
];

const EMPTY_REQUIREMENT: RequirementInput = {
  name: "",
  value_text: "",
  value_num: null,
  unit: "",
  data_type: "STRING",
  mandatory: false,
  range_min: null,
  range_max: null,
  tolerance: null,
  allowed_values: [],
};

export default function ComponentsTab({ tenderId }: { tenderId: string }): JSX.Element {
  const queryClient = useQueryClient();
  const [selected, setSelected] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
  const [newQuantity, setNewQuantity] = useState("1");
  const [newUnit, setNewUnit] = useState("шт");
  const [newCategory, setNewCategory] = useState("");
  const [requirement, setRequirement] = useState<RequirementInput>(EMPTY_REQUIREMENT);
  const [error, setError] = useState<string | null>(null);

  const componentsQuery = useQuery({
    queryKey: ["components", tenderId],
    queryFn: () => listComponents(tenderId),
    refetchInterval: 10_000,
  });

  const requirementsQuery = useQuery({
    queryKey: ["requirements", selected],
    queryFn: () => listRequirements(selected ?? ""),
    enabled: Boolean(selected),
  });

  const invalidate = (): void => {
    void queryClient.invalidateQueries({ queryKey: ["components", tenderId] });
    void queryClient.invalidateQueries({ queryKey: ["requirements", selected] });
    void queryClient.invalidateQueries({ queryKey: ["tender", tenderId] });
  };

  const createComponentMutation = useMutation({
    mutationFn: () =>
      createComponent(tenderId, {
        name: newName.trim(),
        quantity: Number(newQuantity) || 1,
        unit: newUnit.trim() || "шт",
        category: newCategory.trim(),
      }),
    onSuccess: () => {
      setNewName("");
      setNewCategory("");
      setError(null);
      invalidate();
    },
    onError: (mutationError: Error) => setError(mutationError.message),
  });

  const deleteComponentMutation = useMutation({
    mutationFn: (componentId: string) => deleteComponent(componentId),
    onSuccess: () => {
      setSelected(null);
      invalidate();
    },
    onError: (mutationError: Error) => setError(mutationError.message),
  });

  const createRequirementMutation = useMutation({
    mutationFn: () => createRequirement(selected ?? "", requirement),
    onSuccess: () => {
      setRequirement(EMPTY_REQUIREMENT);
      setError(null);
      invalidate();
    },
    onError: (mutationError: Error) => setError(mutationError.message),
  });

  const deleteRequirementMutation = useMutation({
    mutationFn: (requirementId: string) => deleteRequirement(requirementId),
    onSuccess: () => invalidate(),
    onError: (mutationError: Error) => setError(mutationError.message),
  });

  const components = componentsQuery.data ?? [];
  const requirements = requirementsQuery.data ?? [];

  const onAddComponent = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    if (!newName.trim()) {
      setError("Укажите название позиции");
      return;
    }
    createComponentMutation.mutate();
  };

  const onAddRequirement = (event: FormEvent<HTMLFormElement>): void => {
    event.preventDefault();
    if (!requirement.name.trim()) {
      setError("Укажите название требования");
      return;
    }
    createRequirementMutation.mutate();
  };

  return (
    <div className="space-y-4">
      <form className="card grid gap-3 p-4 md:grid-cols-[2fr_1fr_1fr_1fr_auto]" onSubmit={onAddComponent}>
        <input
          className="input"
          placeholder="Новая позиция"
          value={newName}
          onChange={(event) => setNewName(event.target.value)}
        />
        <input
          className="input"
          type="number"
          min="0.001"
          step="0.001"
          value={newQuantity}
          onChange={(event) => setNewQuantity(event.target.value)}
        />
        <input
          className="input"
          placeholder="Ед."
          value={newUnit}
          onChange={(event) => setNewUnit(event.target.value)}
        />
        <input
          className="input"
          placeholder="Категория"
          value={newCategory}
          onChange={(event) => setNewCategory(event.target.value)}
        />
        <button className="btn-primary" type="submit" disabled={createComponentMutation.isPending}>
          Добавить
        </button>
      </form>

      {error ? <p className="rounded-lg bg-red-50 px-3 py-2 text-sm text-red-700">{error}</p> : null}

      <div className="grid gap-4 lg:grid-cols-2">
        <div className="card">
          <h3 className="border-b border-slate-200 p-4 text-sm font-semibold">Позиции</h3>
          {components.length === 0 ? (
            <p className="p-4 text-sm text-slate-500">
              Позиции появятся после разбора документов — или добавьте их вручную.
            </p>
          ) : (
            <table className="table">
              <thead>
                <tr>
                  <th>№</th>
                  <th>Название</th>
                  <th>Кол-во</th>
                  <th>Требований</th>
                  <th>Статус</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {components.map((component) => (
                  <tr
                    key={component.id}
                    className={selected === component.id ? "bg-brand-50/50" : undefined}
                  >
                    <td>{component.position_no}</td>
                    <td>
                      <button
                        type="button"
                        className="text-left font-medium text-brand-700"
                        onClick={() => setSelected(component.id)}
                      >
                        {component.name}
                      </button>
                      {component.error_message ? (
                        <p className="mt-0.5 text-xs text-red-600">{component.error_message}</p>
                      ) : null}
                    </td>
                    <td>
                      {component.quantity} {component.unit}
                    </td>
                    <td>
                      {component.requirements_count}
                      {component.mandatory_count > 0 ? ` (обяз.: ${component.mandatory_count})` : ""}
                    </td>
                    <td>
                      <StatusBadge status={component.status} />
                    </td>
                    <td className="text-right">
                      <button
                        type="button"
                        className="btn-danger"
                        onClick={() => deleteComponentMutation.mutate(component.id)}
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

        <div className="card">
          <h3 className="border-b border-slate-200 p-4 text-sm font-semibold">
            {selected ? "Требования позиции" : "Выберите позицию"}
          </h3>
          {!selected ? (
            <p className="p-4 text-sm text-slate-500">
              Нажмите на название позиции, чтобы увидеть и дополнить требования.
            </p>
          ) : (
            <>
              <div className="max-h-72 overflow-y-auto">
                {requirements.length === 0 ? (
                  <p className="p-4 text-sm text-slate-500">Требований нет.</p>
                ) : (
                  <table className="table">
                    <thead>
                      <tr>
                        <th>Требование</th>
                        <th>Значение</th>
                        <th>Тип</th>
                        <th />
                      </tr>
                    </thead>
                    <tbody>
                      {requirements.map((item) => (
                        <tr key={item.id}>
                          <td>
                            {item.name}
                            {item.mandatory ? (
                              <span className="ml-1 text-xs text-red-600">*</span>
                            ) : null}
                          </td>
                          <td>
                            {item.value_text || item.value_num || "—"} {item.unit}
                          </td>
                          <td className="text-xs text-slate-500">{item.data_type}</td>
                          <td className="text-right">
                            <button
                              type="button"
                              className="text-xs text-red-600"
                              onClick={() => deleteRequirementMutation.mutate(item.id)}
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

              <form className="grid gap-2 border-t border-slate-200 p-4" onSubmit={onAddRequirement}>
                <div className="grid gap-2 sm:grid-cols-2">
                  <input
                    className="input"
                    placeholder="Название"
                    value={requirement.name}
                    onChange={(event) =>
                      setRequirement({ ...requirement, name: event.target.value })
                    }
                  />
                  <input
                    className="input"
                    placeholder="Значение"
                    value={requirement.value_text}
                    onChange={(event) =>
                      setRequirement({ ...requirement, value_text: event.target.value })
                    }
                  />
                </div>
                <div className="grid gap-2 sm:grid-cols-3">
                  <input
                    className="input"
                    placeholder="Ед. изм."
                    value={requirement.unit}
                    onChange={(event) =>
                      setRequirement({ ...requirement, unit: event.target.value })
                    }
                  />
                  <select
                    className="input"
                    value={requirement.data_type}
                    onChange={(event) =>
                      setRequirement({ ...requirement, data_type: event.target.value })
                    }
                  >
                    {DATA_TYPES.map((type) => (
                      <option key={type} value={type}>
                        {type}
                      </option>
                    ))}
                  </select>
                  <label className="flex items-center gap-2 text-sm text-slate-600">
                    <input
                      type="checkbox"
                      checked={requirement.mandatory}
                      onChange={(event) =>
                        setRequirement({ ...requirement, mandatory: event.target.checked })
                      }
                    />
                    Обязательное
                  </label>
                </div>
                <button
                  className="btn-secondary"
                  type="submit"
                  disabled={createRequirementMutation.isPending}
                >
                  Добавить требование
                </button>
              </form>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
