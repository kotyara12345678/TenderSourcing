//! Matching Engine crate.
//!
//! * [`matcher`] — pure, deterministic technical matcher: every requirement
//!   is checked individually, every mismatch is explained, and missing data
//!   is never reported as a match.
//! * [`ranking`] — explainable weighted ranking with explicit notes for
//!   unknown factors (never invented values).
//! * [`server`] — gRPC `MatchingService`.

pub mod tender {
    pub mod common {
        pub mod v1 {
            pub use tender_common::tender::common::v1::*;
        }
    }
    pub mod matching {
        pub mod v1 {
            tonic::include_proto!("tender.matching.v1");
        }
    }
}

pub mod matcher;
pub mod ranking;
pub mod server;
