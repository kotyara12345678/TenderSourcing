//! Search Engine crate.
//!
//! * [`client`] — Keenable HTTP client (`POST /v1/search`, `GET /v1/fetch`)
//!   with the org rate limit and `Retry-After` handling.
//! * [`fetch`] — fetch backends: Keenable (default) and a built-in direct
//!   crawler with SSRF protections.
//! * [`queries`] — deterministic query generation and relevance scoring.
//! * [`jobs`] — the `jobs:search` consumer-group worker that searches,
//!   prefilters, fetches and persists one tender position.
//! * [`server`] — gRPC `SearchService` for synchronous calls.

pub mod tender {
    pub mod common {
        pub mod v1 {
            pub use tender_common::tender::common::v1::*;
        }
    }
    pub mod search {
        pub mod v1 {
            tonic::include_proto!("tender.search.v1");
        }
    }
}

pub mod client;
pub mod fetch;
pub mod jobs;
pub mod queries;
pub mod ratelimit;
pub mod server;
