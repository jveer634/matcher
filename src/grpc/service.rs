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

        let symbol = req.symbol.trim();
        if symbol.is_empty() {
            return Err(Status::invalid_argument("Symbol cannot be empty"));
        }

        let user_id = req.user_id.trim();
        if user_id.is_empty() {
            return Err(Status::invalid_argument("user_id cannot be empty"));
        }

        let order_id = if req.order_id.trim().is_empty() {
            format!(
                "ord-{}",
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
            )
        } else {
            req.order_id.trim().to_string()
        };

        let side = Side::try_from(req.side).map_err(Status::invalid_argument)?;
        let order_type = OrderType::try_from(req.order_type).map_err(Status::invalid_argument)?;

        let quantity = UDecimal::try_from(req.quantity)
            .map_err(|e| Status::invalid_argument(format!("Invalid quantity: {}", e)))?;
        if quantity.is_zero() {
            return Err(Status::invalid_argument(
                "Quantity must be strictly positive",
            ));
        }

        let price = match req.price {
            Some(p) => {
                let dec = UDecimal::try_from(p)
                    .map_err(|e| Status::invalid_argument(format!("Invalid price: {}", e)))?;
                Some(dec)
            }
            None => None,
        };

        let match_res = self
            .engine
            .submit_order(
                order_id.clone(),
                user_id.to_string(),
                symbol,
                side,
                order_type,
                price,
                quantity,
            )
            .map_err(Status::internal)?;

        let current_order = self
            .engine
            .get_order(symbol, &order_id)
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

        if req.symbol.trim().is_empty() {
            return Err(Status::invalid_argument("Symbol cannot be empty"));
        }
        if req.order_id.trim().is_empty() {
            return Err(Status::invalid_argument("order_id cannot be empty"));
        }

        let order = self
            .engine
            .cancel_order(&req.symbol, &req.order_id)
            .map_err(Status::not_found)?;

        Ok(Response::new(CancelOrderResponse {
            order: Some(ProtoOrder::from(&order)),
        }))
    }

    async fn get_depth(
        &self,
        request: Request<GetDepthRequest>,
    ) -> Result<Response<GetDepthResponse>, Status> {
        let req = request.into_inner();
        let symbol = req.symbol.trim();
        if symbol.is_empty() {
            return Err(Status::invalid_argument("Symbol cannot be empty"));
        }

        let limit = if req.limit == 0 {
            50
        } else {
            req.limit as usize
        };

        let (bids, asks) = self
            .engine
            .get_depth(symbol, limit)
            .map_err(Status::not_found)?;

        let bids_proto = bids.into_iter().map(ProtoBookLevel::from).collect();
        let asks_proto = asks.into_iter().map(ProtoBookLevel::from).collect();

        Ok(Response::new(GetDepthResponse {
            symbol: symbol.to_string(),
            bids: bids_proto,
            asks: asks_proto,
        }))
    }

    async fn get_stats(
        &self,
        request: Request<GetStatsRequest>,
    ) -> Result<Response<GetStatsResponse>, Status> {
        let req = request.into_inner();
        let symbol = req.symbol.trim();
        if symbol.is_empty() {
            return Err(Status::invalid_argument("Symbol cannot be empty"));
        }

        let stats = self.engine.get_stats(symbol).map_err(Status::not_found)?;

        Ok(Response::new(GetStatsResponse {
            stats: Some(ProtoStats::from(&stats)),
        }))
    }

    async fn get_trade_history(
        &self,
        request: Request<GetTradeHistoryRequest>,
    ) -> Result<Response<GetTradeHistoryResponse>, Status> {
        let req = request.into_inner();
        let symbol = req.symbol.trim();
        if symbol.is_empty() {
            return Err(Status::invalid_argument("Symbol cannot be empty"));
        }

        let trades = self
            .engine
            .get_trade_history(symbol)
            .map_err(Status::not_found)?;

        let trades_proto = trades.into_iter().map(ProtoTrade::from).collect();

        Ok(Response::new(GetTradeHistoryResponse {
            symbol: symbol.to_string(),
            trades: trades_proto,
        }))
    }

    async fn register_symbol(
        &self,
        request: Request<RegisterSymbolRequest>,
    ) -> Result<Response<RegisterSymbolResponse>, Status> {
        let req = request.into_inner();
        let symbol = req.symbol.trim();
        if symbol.is_empty() {
            return Err(Status::invalid_argument("Symbol cannot be empty"));
        }

        let listing_price =
            match req.listing_price {
                Some(p) => Some(UDecimal::try_from(p).map_err(|e| {
                    Status::invalid_argument(format!("Invalid listing price: {}", e))
                })?),
                None => None,
            };

        match self.engine.register_symbol(symbol, listing_price) {
            Ok(()) => Ok(Response::new(RegisterSymbolResponse {
                success: true,
                message: format!("Symbol '{}' registered successfully", symbol),
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
