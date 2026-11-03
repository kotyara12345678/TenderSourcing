//! Orchestrator crate: the only writer of the tender FSM.
//!
//! It consumes `jobs:control` (API commands) and the `events:*` streams
//! produced by workers, advances the tender state machine, drives the next
//! pipeline stage, performs matching/ranking over gRPC and enforces the
//! overall run watchdog.

pub mod tender {
    pub mod common {
        pub mod v1 {
            pub use tender_common::tender::common::v1::*;
        }
    }
    pub mod documents {
        pub mod v1 {
            tonic::include_proto!("tender.documents.v1");
        }
    }
    pub mod ai {
        pub mod v1 {
            tonic::include_proto!("tender.ai.v1");
        }
    }
    pub mod search {
        pub mod v1 {
            tonic::include_proto!("tender.search.v1");
        }
    }
    pub mod reports {
        pub mod v1 {
            tonic::include_proto!("tender.reports.v1");
        }
    }
    pub mod matching {
        pub mod v1 {
            tonic::include_proto!("tender.matching.v1");
        }
    }
}

pub mod control;
pub mod fsm;
pub mod handlers;
pub mod state;
