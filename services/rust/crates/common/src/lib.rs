//! Shared foundation for all TenderSourcing Rust services.
//!
//! * [`config`] — centralized ENV validation, the single source of truth for
//!   every Rust service (variables documented in `.env.example`).
//! * [`error`] — unified error type with HTTP (axum) and gRPC (tonic) mappings.
//! * [`telemetry`] — tracing initialization.
//! * [`streams`] — Redis Streams job/event topology and helpers.
//! * [`progress`] — `ProgressEvent` construction and publication.
//! * [`tender::common::v1`] — generated protobuf types (job envelope,
//!   progress events, health) shared across services.

pub mod config;
pub mod error;
pub mod progress;
pub mod streams;
pub mod telemetry;

/// Generated types from `proto/common/v1/common.proto`, exposed at the
/// canonical package path so prost-generated cross-package references in
/// other service crates resolve to these exact types via re-export.
pub mod tender {
    pub mod common {
        pub mod v1 {
            tonic::include_proto!("tender.common.v1");
        }
    }
}
