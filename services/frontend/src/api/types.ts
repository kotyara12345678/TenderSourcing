// Types mirror the JSON shapes returned by services/rust/crates/api.

export type TenderStatus =
  | "CREATED"
  | "UPLOADING"
  | "DOCUMENT_PROCESSING"
  | "REQUIREMENT_EXTRACTION"
  | "REQUIREMENTS_REVIEW"
  | "SEARCH_QUEUED"
  | "SEARCHING"
  | "FETCHING"
  | "PRODUCT_EXTRACTION"
  | "MATCHING"
  | "RANKING"
  | "REPORT_GENERATION"
  | "COMPLETED"
  | "COMPLETED_WITH_ERRORS"
  | "FAILED"
  | "CANCELLED";

export interface Page<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

export interface TenderSummary {
  id: string;
  name: string;
  description: string;
  status: TenderStatus;
  error_message: string | null;
  created_at: string;
  updated_at: string;
  components_count: number;
}

export interface RunSummary {
  id: string;
  status: string;
  components_total: number;
  components_done: number;
  components_failed: number;
  queries_issued: number;
  results_found: number;
  pages_fetched: number;
  started_at: string | null;
  finished_at: string | null;
  created_at: string;
}

export interface TenderDetail extends TenderSummary {
  documents_count: number;
  reports_count: number;
  latest_run: RunSummary | null;
}

export interface DocumentView {
  id: string;
  tender_id: string;
  original_filename: string;
  mime_type: string;
  size_bytes: number;
  status: string;
  ai_status: string;
  error_message: string | null;
  page_count: number | null;
  ocr_used: boolean;
  uploaded_at: string;
  processed_at: string | null;
}

export interface ComponentView {
  id: string;
  position_no: number;
  name: string;
  quantity: number;
  unit: string;
  category: string;
  status: string;
  source: string;
  error_message: string | null;
  requirements_count: number;
  mandatory_count: number;
}

export interface RequirementView {
  id: string;
  component_id: string;
  name: string;
  value_text: string;
  value_num: number | null;
  unit: string;
  data_type: string;
  mandatory: boolean;
  range_min: number | null;
  range_max: number | null;
  tolerance: number | null;
  allowed_values: string[];
  source: string;
  confidence: number;
}

export interface ComponentInput {
  name: string;
  quantity: number;
  unit: string;
  category: string;
}

export interface RequirementInput {
  name: string;
  value_text: string;
  value_num: number | null;
  unit: string;
  data_type: string;
  mandatory: boolean;
  range_min: number | null;
  range_max: number | null;
  tolerance: number | null;
  allowed_values: string[];
}

export interface MatchView {
  component_id: string;
  match_id: string;
  verdict: string;
  eligible: boolean;
  technical_score: number;
  rank: number | null;
  total_score: number | null;
  explanation: string;
  product_id: string;
  product_name: string;
  manufacturer: string;
  model: string;
  price: number | null;
  currency: string | null;
  lead_time_days: number | null;
  canonical_url: string;
  supplier_name: string | null;
  supplier_url: string | null;
}

export interface MatchDetailView {
  match_id: string;
  requirement_name: string;
  status: string;
  expected: string;
  actual: string;
  unit: string;
  explanation: string;
}

export interface ComponentResults {
  id: string;
  position_no: number;
  name: string;
  quantity: number;
  unit: string;
  status: string;
  matches: MatchView[];
}

export interface ComponentMatchBundle {
  component_id: string;
  matches: MatchView[];
  details: MatchDetailView[];
}

export interface ReportView {
  id: string;
  tender_id: string;
  run_id: string | null;
  format: "xlsx" | "pdf" | "docx";
  size_bytes: number;
  status: string;
  error_message: string | null;
  created_at: string;
  ready_at: string | null;
}

export interface ActivityView {
  id: string;
  tender_id: string | null;
  event_type: string;
  message: string;
  metadata: Record<string, unknown>;
  created_at: string;
}

export interface UsageByService {
  service: string;
  calls: number;
  credits: number;
  tokens_in: number;
  tokens_out: number;
  cost_usd: number;
}

export interface UsageByTender {
  tender_id: string;
  tender_name: string;
  calls: number;
  credits: number;
  cost_usd: number;
}

export interface RecentUsage {
  id: string;
  service: string;
  sku: string;
  tender_id: string | null;
  quantity: number;
  credits: number;
  cost_usd: number;
  created_at: string;
}

export interface UsageSummary {
  totals: {
    calls: number;
    credits: number;
    tokens_in: number;
    tokens_out: number;
    cost_usd: number;
  };
  by_service: UsageByService[];
  by_tender: UsageByTender[];
  recent: RecentUsage[];
}

export interface UserView {
  id: string;
  username: string;
  display_name: string;
  csrf_token: string;
  expires_at: string;
}

export interface LoginResponse {
  user: UserView;
}

export interface StartSearchResponse {
  run_id: string;
  positions: number;
}

// ---- AI intake (free-form request -> positions) ----

export type IntakeStatus = "QUEUED" | "RUNNING" | "COMPLETED" | "FAILED" | "APPLIED";

export type IntakeStepStatus = "pending" | "running" | "done" | "error";

export interface IntakeStep {
  key: string;
  title: string;
  status: IntakeStepStatus;
  detail: string;
}

export interface IntakeRequirement {
  name: string;
  value_text: string;
  value_num: number | null;
  unit: string;
  data_type: string;
  mandatory: boolean;
}

export interface IntakePosition {
  name: string;
  quantity: number;
  unit: string;
  category: string;
  requirements: IntakeRequirement[];
}

export interface IntakeResult {
  tender_name: string;
  summary: string;
  positions: IntakePosition[];
}

export interface IntakeView {
  id: string;
  status: IntakeStatus;
  progress: number;
  steps: IntakeStep[];
  result: IntakeResult | null;
  error_message: string | null;
  tender_id: string | null;
}

export interface ApplyPosition {
  name: string;
  quantity: number;
  unit: string;
  category: string;
  requirements: IntakeRequirement[];
}

export interface ApplyIntakeResponse {
  tender_id: string;
  positions: number;
  requirements: number;
}
