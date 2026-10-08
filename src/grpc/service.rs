use std::sync::Arc;
use tonic::{Request, Response, Status};

use crate::engine::ShardedEngine;
use crate::gen::engine::v1::matcher_service_server::MatcherService;
use crate::gen::engine::v1::{
    CancelOrderRequest, CancelOrderResponse, GetDepthRequest, GetDepthResponse, GetStatsRequest,
    GetStatsResponse, GetTradeHistoryRequest, GetTradeHistoryResponse, ListSymbolsRequest,
    ListSymbolsResponse, RegisterSymbolRequest, RegisterSymbolResponse, SubmitOrderRequest,
    SubmitOrderResponse,
};
use crate::gen::order::v1::Order as ProtoOrder;
use crate::gen::orderbook::v1::{BookLevel as ProtoBookLevel, Stats as ProtoStats};
use crate::gen::trade::v1::Trade as ProtoTrade;
use crate::orderbook::{OrderType, Side, UDecimal};

#[derive(Debug, Clone)]
pub struct MatcherServiceImpl {
    engine: Arc<ShardedEngine>,
}

impl MatcherServiceImpl {
    pub fn new(engine: Arc<ShardedEngine>) -> Self {
        Self { engine }
    }
}

#[tonic::async_trait]
impl MatcherService for MatcherServiceImpl {
    async fn submit_order(
        &self,
        request: Request<SubmitOrderRequest>,
    ) -> Result<Response<SubmitOrderResponse>, Status> {
        let req = request.into_inner();

        let side = Side::try_from(req.side).map_err(Status::invalid_argument)?;
        let order_type = OrderType::try_from(req.order_type).map_err(Status::invalid_argument)?;
        let quantity = UDecimal::try_from(req.quantity)
            .map_err(|e| Status::invalid_argument(format!("Invalid quantity: {e}")))?;

        let price = match req.price {
            Some(p) => Some(
                UDecimal::try_from(p)
                    .map_err(|e| Status::invalid_argument(format!("Invalid price: {e}")))?,
            ),
            None => None,
        };

        let match_res = self
            .engine
            .submit_order(
                req.order_id.clone(),
                req.user_id,
                &req.symbol,
                side,
                order_type,
                price,
                quantity,
            )
            .map_err(|e| {
                if e.contains("Unknown symbol") {
                    Status::not_found(e)
                } else {
                    Status::invalid_argument(e)
                }
            })?;

        let current_order = self
            .engine
            .get_order(&req.symbol, &req.order_id)
            .unwrap_or(None)
            .map(|o| ProtoOrder::from(&o));

        let trades_proto: Vec<ProtoTrade> =
            match_res.trades.into_iter().map(ProtoTrade::from).collect();

        Ok(Response::new(SubmitOrderResponse {
            order_id: match_res.order_id,
            trades: trades_proto,
            order: current_order,
        }))
    }

    async fn cancel_order(
        &self,
        request: Request<CancelOrderRequest>,
    ) -> Result<Response<CancelOrderResponse>, Status> {
        let req = request.into_inner();

        let order = self
            .engine
            .cancel_order(&req.symbol, &req.order_id, &req.user_id)
            .map_err(|err| {
                if err.contains("Unauthorized") {
                    Status::permission_denied(err)
                } else if err.contains("not found") || err.contains("Unknown symbol") {
                    Status::not_found(err)
                } else {
                    Status::invalid_argument(err)
                }
            })?;

        Ok(Response::new(CancelOrderResponse {
            order: Some(ProtoOrder::from(&order)),
        }))
    }

    async fn get_depth(
        &self,
        request: Request<GetDepthRequest>,
    ) -> Result<Response<GetDepthResponse>, Status> {
        let req = request.into_inner();

        let (bids, asks) = self
            .engine
            .get_depth(&req.symbol, req.limit as usize)
            .map_err(Status::not_found)?;

        let bids_proto = bids.into_iter().map(ProtoBookLevel::from).collect();
        let asks_proto = asks.into_iter().map(ProtoBookLevel::from).collect();

        Ok(Response::new(GetDepthResponse {
            symbol: req.symbol,
            bids: bids_proto,
            asks: asks_proto,
        }))
    }

    async fn get_stats(
        &self,
        request: Request<GetStatsRequest>,
    ) -> Result<Response<GetStatsResponse>, Status> {
        let req = request.into_inner();

        let stats = self
            .engine
            .get_stats(&req.symbol)
            .map_err(Status::not_found)?;

        Ok(Response::new(GetStatsResponse {
            stats: Some(ProtoStats::from(&stats)),
        }))
    }

    async fn get_trade_history(
        &self,
        request: Request<GetTradeHistoryRequest>,
    ) -> Result<Response<GetTradeHistoryResponse>, Status> {
        let req = request.into_inner();

        let trades = self
            .engine
            .get_trade_history(&req.symbol)
            .map_err(Status::not_found)?;

        let trades_proto = trades.into_iter().map(ProtoTrade::from).collect();

        Ok(Response::new(GetTradeHistoryResponse {
            symbol: req.symbol,
            trades: trades_proto,
        }))
    }

    async fn register_symbol(
        &self,
        request: Request<RegisterSymbolRequest>,
    ) -> Result<Response<RegisterSymbolResponse>, Status> {
        let req = request.into_inner();

        let listing_price = match req.listing_price {
            Some(p) => Some(
                UDecimal::try_from(p)
                    .map_err(|e| Status::invalid_argument(format!("Invalid listing price: {e}")))?,
            ),
            None => None,
        };

        match self.engine.register_symbol(&req.symbol, listing_price) {
            Ok(()) => Ok(Response::new(RegisterSymbolResponse {
                success: true,
                message: format!("Symbol '{}' registered successfully", req.symbol),
            })),
            Err(e) => Err(Status::already_exists(e)),
        }
    }

    async fn list_symbols(
        &self,
        _request: Request<ListSymbolsRequest>,
    ) -> Result<Response<ListSymbolsResponse>, Status> {
        let symbols = self.engine.list_symbols();
        Ok(Response::new(ListSymbolsResponse { symbols }))
    }
}
