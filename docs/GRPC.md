# gRPC contracts

`proto/` is the single source of truth; generated code is never committed.

| File | Package | Used by |
| --- | --- | --- |
| `proto/common/v1/common.proto` | `tender.common.v1` | everyone (`JobEnvelope`, `ProgressEvent`, `Health*`) |
| `proto/documents/v1/document.proto` | `tender.documents.v1` | API → document worker (`jobs:documents`) |
| `proto/ai/v1/ai.proto` | `tender.ai.v1` | orchestrator → AI worker, AI worker → `events:ai` |
| `proto/search/v1/search.proto` | `tender.search.v1` | orchestrator/API → search (`SearchService`), `jobs:search` |
| `proto/matching/v1/matching.proto` | `tender.matching.v1` | orchestrator → matching (`MatchingService`) |
| `proto/reports/v1/report.proto` | `tender.reports.v1` | orchestrator → report worker (`jobs:reports`) |

## Services

```proto
service SearchService {
  rpc Search(SearchRequest) returns (SearchResponse);
  rpc Fetch(FetchRequest)  returns (FetchResponse);
  rpc Health(HealthRequest) returns (HealthResponse);
}

service MatchingService {
  rpc MatchComponent(MatchComponentRequest) returns (MatchComponentResponse);
  rpc RankOffers(RankOffersRequest)         returns (RankOffersResponse);
  rpc Health(HealthRequest)                 returns (HealthResponse);
}
```

Endpoints: `search` listens on `:50052`, `matching` on `:50051`
(`SEARCH_GRPC_URL`, `MATCHING_GRPC_URL`). Neither is published to the host; they
are reachable only inside the Compose network.

## Verification semantics

`MatchVerdict` follows the contract:

| Verdict | Meaning | Eligible |
| --- | --- | --- |
| `FULL` | every evaluable requirement matches | yes |
| `TOLERANCE` | numeric values deviate within the requirement tolerance | yes |
| `PARTIAL` | non-mandatory checks are partial/missing; mandatory pass | yes |
| `MISMATCH` | at least one mandatory requirement fails on data | no |
| `INSUFFICIENT` | a mandatory requirement has no product data | no |

`RequirementCheck.status ∈ {match, tolerance, partial, mismatch, missing}` and
every non-match carries a human-readable `explanation`.

## Job envelopes

Every stream entry carries a `JobEnvelope`:

```proto
message JobEnvelope {
  string task_id = 1;   // idempotency key
  string tender_id = 2;
  string run_id = 3;    // empty during the document stage
  int64 created_at_unix_ms = 4;
  int32 attempt = 5;
  map<string,string> metadata = 6;  // action for jobs:control
}
```

`jobs:control` actions: `start_run`, `cancel`, `regenerate_report`,
`documents_uploaded`.

## Regeneration

```bash
# Rust (protoc via build scripts, requires protobuf-compiler locally)
cargo build --manifest-path services/rust/Cargo.toml

# Python (never committed; generated dir is git-ignored)
bash scripts/gen-proto-python.sh
```
