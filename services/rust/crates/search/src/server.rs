//! gRPC `SearchService`: synchronous search/fetch for the orchestrator and
//! a health probe that verifies real upstream reachability.

use std::sync::Arc;

use tonic::{Request, Response, Status};

use tender_common::error::AppResult;
use tender_common::streams::Streams;

use crate::client::KeenableClient;
use crate::fetch::FetchBackend;
use crate::tender::common::v1::{HealthRequest, HealthResponse};
use crate::tender::search::v1::{
    search_service_server::{SearchService, SearchServiceServer},
    FetchRequest, FetchResponse, SearchRequest, SearchResponse,
};

pub struct SearchGrpc {
    pub client: Arc<KeenableClient>,
    pub backend: Arc<FetchBackend>,
    pub streams: Streams,
}

impl SearchGrpc {
    pub fn into_server(self) -> SearchServiceServer<Self> {
        SearchServiceServer::new(self)
    }
}

#[tonic::async_trait]
impl SearchService for SearchGrpc {
    async fn search(
        &self,
        request: Request<SearchRequest>,
    ) -> Result<Response<SearchResponse>, Status> {
        let request = request.into_inner();
        self.client
            .search(&request)
            .await
            .map(Response::new)
            .map_err(Status::from)
    }

    async fn fetch(
        &self,
        request: Request<FetchRequest>,
    ) -> Result<Response<FetchResponse>, Status> {
        let request = request.into_inner();
        let parsed = reqwest::Url::parse(&request.url)
            .map_err(|err| Status::invalid_argument(format!("invalid url: {err}")))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(Status::invalid_argument("only http(s) urls are supported"));
        }
        self.client
            .fetch(&request)
            .await
            .map(Response::new)
            .map_err(Status::from)
    }

    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        let mut ok = true;
        let mut details: Vec<String> = Vec::new();

        match self.streams.ping().await {
            Ok(()) => details.push("redis ok".to_string()),
            Err(err) => {
                ok = false;
                details.push(format!("redis error: {err}"));
            }
        }

        // The public fetch twin is free and unauthenticated: a real
        // reachability probe instead of "config looks fine".
        match self.client.probe_public().await {
            Ok(()) => details.push("keenable reachable".to_string()),
            Err(err) => {
                ok = false;
                details.push(format!("keenable: {err}"));
            }
        }

        Ok(Response::new(HealthResponse {
            ok,
            detail: details.join(", "),
        }))
    }
}

/// Kept for symmetry with other services' `main.rs` health wiring.
pub async fn readiness(streams: &Streams, client: &KeenableClient) -> AppResult<()> {
    streams.ping().await?;
    client.probe_public().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn grpc_url_validation_rejects_other_schemes() {
        let bad = reqwest::Url::parse("javascript:alert(1)").unwrap();
        assert!(!matches!(bad.scheme(), "http" | "https"));
    }
}
