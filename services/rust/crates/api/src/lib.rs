//! API Gateway crate.
//!
//! * [`auth`] — ENV-bootstrapped operator account, argon2id hashing, HttpOnly
//!   session cookies (`ts_session`), HMAC password fingerprint, CSRF tokens.
//! * [`storage`] — uploaded document storage (local filesystem in v1).
//! * [`routes`] — REST endpoints under `/api/v1`.
//! * [`ws`] — WebSocket progress fan-out from the `events:progress` channel.

pub mod auth;
pub mod routes;
pub mod state;
pub mod storage;
pub mod ws;

/// Generated protobuf types used by the API (job envelope, progress events,
/// document extraction jobs).
pub mod tender {
    pub mod common {
        pub mod v1 {
            tonic::include_proto!("tender.common.v1");
        }
    }
    pub mod documents {
        pub mod v1 {
            tonic::include_proto!("tender.documents.v1");
        }
    }
}
