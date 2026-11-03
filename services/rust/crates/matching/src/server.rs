//! gRPC `MatchingService`: technical matching and ranking.

use tonic::{Request, Response, Status};

use crate::matcher::match_component;
use crate::ranking::rank_component;
use crate::tender::common::v1::{HealthRequest, HealthResponse};
use crate::tender::matching::v1::{
    matching_service_server::{MatchingService, MatchingServiceServer},
    MatchComponentRequest, MatchComponentResponse, RankOffersRequest, RankOffersResponse,
};

#[derive(Default)]
pub struct MatchingGrpc;

impl MatchingGrpc {
    pub fn into_server(self) -> MatchingServiceServer<Self> {
        MatchingServiceServer::new(self)
    }
}

#[tonic::async_trait]
impl MatchingService for MatchingGrpc {
    async fn match_component(
        &self,
        request: Request<MatchComponentRequest>,
    ) -> Result<Response<MatchComponentResponse>, Status> {
        let request = request.into_inner();
        if request.component_id.trim().is_empty() {
            return Err(Status::invalid_argument("component_id is required"));
        }
        let matches = match_component(
            &request.component_id,
            &request.requirements,
            &request.products,
        );
        Ok(Response::new(MatchComponentResponse { matches }))
    }

    async fn rank_offers(
        &self,
        request: Request<RankOffersRequest>,
    ) -> Result<Response<RankOffersResponse>, Status> {
        let request = request.into_inner();
        if request.component_id.trim().is_empty() {
            return Err(Status::invalid_argument("component_id is required"));
        }
        let offers = rank_component(&request);
        Ok(Response::new(RankOffersResponse { offers }))
    }

    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        // The matcher is pure and in-process: nothing external to probe.
        Ok(Response::new(HealthResponse {
            ok: true,
            detail: "in-process matcher and ranker".to_string(),
        }))
    }
}
