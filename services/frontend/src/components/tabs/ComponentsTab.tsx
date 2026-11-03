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
  const selectedComponent = components.find((item) => item.id === selected) ?? null;

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
      <form className="panel" onSubmit={onAddComponent}>
        <div className="panel-body grid gap-2 md:grid-cols-[1.6fr_0.6fr_0.5fr_1fr_auto]">
          <input
            className="input"
            placeholder="Новая позиция"
            value={newName}
            onChange={(event) => setNewName(event.target.value)}
          />
          <input
            className="input num"
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
          <button className="btn-secondary" type="submit" disabled={createComponentMutation.isPending}>
            Добавить
          </button>
        </div>
      </form>

      {error ? <p className="banner-error">{error}</p> : null}

      <div className="grid gap-4 lg:grid-cols-2">
        <div className="panel">
          <div className="panel-head">
            <span className="section-title">Позиции</span>
            <span className="text-[12px] tabular-nums text-zinc-400">{components.length}</span>
          </div>
          {componentsQuery.isPending ? (
            <p className="empty">Загрузка…</p>
          ) : components.length === 0 ? (
            <p className="empty">
              Позиции появятся после разбора документов — или добавьте их вручную.
            </p>
          ) : (
            <table className="table table-tight">
              <thead>
                <tr>
                  <th className="w-10 text-right">№</th>
                  <th>Название</th>
                  <th className="w-24 text-right">Кол-во</th>
                  <th className="w-16 text-right">Треб.</th>
                  <th className="w-32">Статус</th>
                  <th className="w-16" />
                </tr>
              </thead>
              <tbody>
                {components.map((component) => (
                  <tr
                    key={component.id}
                    className={selected === component.id ? "bg-zinc-50" : undefined}
                  >
                    <td className="num text-right text-zinc-400">{component.position_no}</td>
                    <td>
                      <button
                        type="button"
                        className={`text-left text-[13px] ${
                          selected === component.id
                            ? "font-medium text-zinc-900"
                            : "text-zinc-700 hover:text-zinc-900"
                        }`}
                        onClick={() => setSelected(component.id)}
                      >
                        {component.name}
                      </button>
                      {component.error_message ? (
                        <p className="mt-0.5 text-[12px] text-red-700">{component.error_message}</p>
                      ) : null}
                    </td>
                    <td className="num whitespace-nowrap text-right">
                      {component.quantity} {component.unit}
                    </td>
                    <td className="num text-right">
                      {component.requirements_count}
                      {component.mandatory_count > 0 ? (
                        <span className="ml-1 text-[11px] text-zinc-400">
                          /{component.mandatory_count}
                        </span>
                      ) : null}
                    </td>
                    <td>
                      <StatusBadge status={component.status} />
                    </td>
                    <td className="text-right">
                      <button
                        type="button"
                        className="btn-danger btn-sm"
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

        <div className="panel">
          <div className="panel-head">
            <span className="section-title">
              {selectedComponent ? selectedComponent.name : "Требования позиции"}
            </span>
            {selectedComponent ? (
              <span className="text-[12px] tabular-nums text-zinc-400">{requirements.length}</span>
            ) : null}
          </div>

          {!selected ? (
            <p className="empty">Выберите позицию слева, чтобы увидеть и дополнить требования.</p>
          ) : (
            <>
              <div className="max-h-72 overflow-y-auto">
                {requirementsQuery.isPending ? (
                  <p className="empty">Загрузка…</p>
                ) : requirements.length === 0 ? (
                  <p className="empty">Требований нет.</p>
                ) : (
                  <table className="table table-tight">
                    <thead>
                      <tr>
                        <th>Требование</th>
                        <th className="w-32">Значение</th>
                        <th className="w-24">Тип</th>
                        <th className="w-16" />
                      </tr>
                    </thead>
                    <tbody>
                      {requirements.map((item) => (
                        <tr key={item.id}>
                          <td>
                            {item.name}
                            {item.mandatory ? (
                              <span className="ml-1 text-[12px] text-red-600" title="Обязательное">
                                *
                              </span>
                            ) : null}
                          </td>
                          <td className="whitespace-nowrap">
                            {item.value_text || item.value_num || "—"} {item.unit}
                          </td>
                          <td className="mono text-zinc-500">{item.data_type}</td>
                          <td className="text-right">
                            <button
                              type="button"
                              className="btn-ghost btn-sm text-red-700 hover:bg-red-50"
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

              <form className="grid gap-2 border-t border-zinc-200 px-4 py-3" onSubmit={onAddRequirement}>
                <div className="grid gap-2 sm:grid-cols-2">
                  <input
                    className="input input-sm"
                    placeholder="Название"
                    value={requirement.name}
                    onChange={(event) =>
                      setRequirement({ ...requirement, name: event.target.value })
                    }
                  />
                  <input
                    className="input input-sm"
                    placeholder="Значение"
                    value={requirement.value_text}
                    onChange={(event) =>
                      setRequirement({ ...requirement, value_text: event.target.value })
                    }
                  />
                </div>
                <div className="grid items-center gap-2 sm:grid-cols-[1fr_1fr_auto_auto]">
                  <input
                    className="input input-sm"
                    placeholder="Ед. изм."
                    value={requirement.unit}
                    onChange={(event) =>
                      setRequirement({ ...requirement, unit: event.target.value })
                    }
                  />
                  <select
                    className="select input-sm"
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
                  <label className="flex items-center gap-1.5 whitespace-nowrap text-[12px] text-zinc-600">
                    <input
                      type="checkbox"
                      className="checkbox"
                      checked={requirement.mandatory}
                      onChange={(event) =>
                        setRequirement({ ...requirement, mandatory: event.target.checked })
                      }
                    />
                    Обязательное
                  </label>
                  <button
                    className="btn-secondary btn-sm"
                    type="submit"
                    disabled={createRequirementMutation.isPending}
                  >
                    Добавить
                  </button>
                </div>
              </form>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
