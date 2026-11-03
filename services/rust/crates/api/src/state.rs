//! Shared API state.

use std::sync::Arc;

use tender_common::config::Config;
use tender_common::streams::Streams;

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub pg: sqlx::PgPool,
    pub streams: Streams,
}
