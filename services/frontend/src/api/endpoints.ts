import { apiFetch } from "@/api/client";
import type {
  ActivityView,
  ApplyIntakeResponse,
  ApplyPosition,
  ComponentInput,
  ComponentMatchBundle,
  ComponentResults,
  ComponentView,
  DocumentView,
  IntakeView,
  LoginResponse,
  Page,
  ReportView,
  RequirementInput,
  RequirementView,
  RunSummary,
  StartSearchResponse,
  TenderDetail,
  TenderSummary,
  UsageSummary,
  UserView,
} from "@/api/types";

// ---- auth ----

export function login(username: string, password: string): Promise<LoginResponse> {
  return apiFetch<LoginResponse>("/auth/login", {
    method: "POST",
    body: JSON.stringify({ username, password }),
  });
}

export function logout(): Promise<{ ok: boolean }> {
  return apiFetch<{ ok: boolean }>("/auth/logout", { method: "POST" });
}

export function me(): Promise<UserView> {
  return apiFetch<UserView>("/auth/me");
}

// ---- tenders ----

export function listTenders(params: {
  page?: number;
  pageSize?: number;
  status?: string;
  q?: string;
}): Promise<Page<TenderSummary>> {
  const search = new URLSearchParams();
  if (params.page) search.set("page", String(params.page));
  if (params.pageSize) search.set("page_size", String(params.pageSize));
  if (params.status) search.set("status", params.status);
  if (params.q) search.set("q", params.q);
  const suffix = search.toString();
  return apiFetch<Page<TenderSummary>>(`/tenders${suffix ? `?${suffix}` : ""}`);
}

export function getTender(id: string): Promise<TenderDetail> {
  return apiFetch<TenderDetail>(`/tenders/${id}`);
}

export function createTender(input: { name: string; description: string }): Promise<TenderSummary> {
  return apiFetch<TenderSummary>("/tenders", {
    method: "POST",
    body: JSON.stringify(input),
  });
}

export function deleteTender(id: string): Promise<{ ok: boolean }> {
  return apiFetch<{ ok: boolean }>(`/tenders/${id}`, { method: "DELETE" });
}

export function cancelTender(id: string): Promise<{ queued: boolean }> {
  return apiFetch<{ queued: boolean }>(`/tenders/${id}/cancel`, { method: "POST" });
}

export function startSearch(id: string): Promise<StartSearchResponse> {
  return apiFetch<StartSearchResponse>(`/tenders/${id}/start-search`, { method: "POST" });
}

// ---- documents ----

export function listDocuments(tenderId: string): Promise<DocumentView[]> {
  return apiFetch<DocumentView[]>(`/tenders/${tenderId}/documents`);
}

export function uploadDocument(tenderId: string, file: File): Promise<DocumentView> {
  const body = new FormData();
  body.append("file", file);
  return apiFetch<DocumentView>(`/tenders/${tenderId}/documents`, {
    method: "POST",
    body,
  });
}

export function deleteDocument(id: string): Promise<{ ok: boolean }> {
  return apiFetch<{ ok: boolean }>(`/documents/${id}`, { method: "DELETE" });
}

// ---- components & requirements ----

export function listComponents(tenderId: string): Promise<ComponentView[]> {
  return apiFetch<ComponentView[]>(`/tenders/${tenderId}/components`);
}

export function createComponent(tenderId: string, input: ComponentInput): Promise<ComponentView> {
  return apiFetch<ComponentView>(`/tenders/${tenderId}/components`, {
    method: "POST",
    body: JSON.stringify(input),
  });
}

export function updateComponent(
  componentId: string,
  input: ComponentInput,
): Promise<ComponentView> {
  return apiFetch<ComponentView>(`/components/${componentId}`, {
    method: "PATCH",
    body: JSON.stringify(input),
  });
}

export function deleteComponent(componentId: string): Promise<{ ok: boolean }> {
  return apiFetch<{ ok: boolean }>(`/components/${componentId}`, { method: "DELETE" });
}

export function listRequirements(componentId: string): Promise<RequirementView[]> {
  return apiFetch<RequirementView[]>(`/components/${componentId}/requirements`);
}

export function createRequirement(
  componentId: string,
  input: RequirementInput,
): Promise<RequirementView> {
  return apiFetch<RequirementView>(`/components/${componentId}/requirements`, {
    method: "POST",
    body: JSON.stringify(input),
  });
}

export function updateRequirement(
  requirementId: string,
  input: RequirementInput,
): Promise<RequirementView> {
  return apiFetch<RequirementView>(`/requirements/${requirementId}`, {
    method: "PATCH",
    body: JSON.stringify(input),
  });
}

export function deleteRequirement(requirementId: string): Promise<{ ok: boolean }> {
  return apiFetch<{ ok: boolean }>(`/requirements/${requirementId}`, { method: "DELETE" });
}

// ---- results & reports ----

export function tenderResults(tenderId: string): Promise<ComponentResults[]> {
  return apiFetch<ComponentResults[]>(`/tenders/${tenderId}/results`);
}

export function componentMatches(componentId: string): Promise<ComponentMatchBundle> {
  return apiFetch<ComponentMatchBundle>(`/components/${componentId}/matches`);
}

export function listRuns(tenderId: string): Promise<RunSummary[]> {
  return apiFetch<RunSummary[]>(`/tenders/${tenderId}/runs`);
}

export function listReports(tenderId: string): Promise<ReportView[]> {
  return apiFetch<ReportView[]>(`/tenders/${tenderId}/reports`);
}

// ---- activity & usage ----

export function tenderActivity(tenderId: string, limit = 50): Promise<ActivityView[]> {
  return apiFetch<ActivityView[]>(`/tenders/${tenderId}/activity?limit=${limit}`);
}

export function recentActivity(limit = 100): Promise<ActivityView[]> {
  return apiFetch<ActivityView[]>(`/activity?limit=${limit}`);
}

export function usageSummary(): Promise<UsageSummary> {
  return apiFetch<UsageSummary>("/usage/summary");
}

// ---- AI intake ----

export function parseIntake(text: string): Promise<{ id: string }> {
  return apiFetch<{ id: string }>("/assistant/parse", {
    method: "POST",
    body: JSON.stringify({ text }),
  });
}

export function getIntake(id: string): Promise<IntakeView> {
  return apiFetch<IntakeView>(`/assistant/${id}`);
}

export function applyIntake(
  id: string,
  input: { name: string; description: string; positions: ApplyPosition[] },
): Promise<ApplyIntakeResponse> {
  return apiFetch<ApplyIntakeResponse>(`/assistant/${id}/apply`, {
    method: "POST",
    body: JSON.stringify(input),
  });
}
