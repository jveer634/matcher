use std::sync::Arc;
use tonic::Request;

use matcher::gen::engine::v1::matcher_service_server::MatcherService;
use matcher::gen::engine::v1::{
    CancelOrderRequest, GetDepthRequest, GetStatsRequest, GetTradeHistoryRequest,
    ListSymbolsRequest, RegisterSymbolRequest, SubmitOrderRequest,
};
use matcher::gen::order::v1::{OrderStatus, OrderType, Side};
use matcher::grpc::MatcherServiceImpl;
use matcher::ShardedEngine;

#[tokio::test]
async fn test_grpc_register_and_list_symbols() {
    let engine = Arc::new(ShardedEngine::new());
    let service = MatcherServiceImpl::new(engine);

    // Register BTC-USDT
    let reg_res = service
        .register_symbol(Request::new(RegisterSymbolRequest {
            symbol: "BTC-USDT".to_string(),
            listing_price: Some(60000.0),
        }))
        .await
        .expect("register symbol");
    assert!(reg_res.into_inner().success);

    // Duplicate registration should fail
    let dup_res = service
        .register_symbol(Request::new(RegisterSymbolRequest {
            symbol: "BTC-USDT".to_string(),
            listing_price: Some(60000.0),
        }))
        .await;
    assert!(dup_res.is_err());

    // Register ETH-USDT
    service
        .register_symbol(Request::new(RegisterSymbolRequest {
            symbol: "ETH-USDT".to_string(),
            listing_price: Some(3000.0),
        }))
        .await
        .unwrap();

    // List symbols
    let list_res = service
        .list_symbols(Request::new(ListSymbolsRequest {}))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(list_res.symbols.len(), 2);
    assert!(list_res.symbols.contains(&"BTC-USDT".to_string()));
    assert!(list_res.symbols.contains(&"ETH-USDT".to_string()));
}

#[tokio::test]
async fn test_grpc_submit_and_match_orders() {
    let engine = Arc::new(ShardedEngine::new());
    let service = MatcherServiceImpl::new(engine);

    // Register symbol
    service
        .register_symbol(Request::new(RegisterSymbolRequest {
            symbol: "BTC-USDT".to_string(),
            listing_price: Some(65000.0),
        }))
        .await
        .unwrap();

    // Submit resting Limit Sell
    let sell_res = service
        .submit_order(Request::new(SubmitOrderRequest {
            order_id: "sell-1".to_string(),
            user_id: "alice".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Sell as i32,
            order_type: OrderType::Limit as i32,
            price: Some(65000.0),
            quantity: 2.0,
        }))
        .await
        .expect("submit limit sell")
        .into_inner();

    assert_eq!(sell_res.order_id, "sell-1");
    assert_eq!(sell_res.trades.len(), 0);
    let sell_order = sell_res.order.expect("order present");
    assert_eq!(sell_order.status, OrderStatus::Open as i32);
    assert_eq!(sell_order.quantity, 2.0);

    // Submit matching Limit Buy
    let buy_res = service
        .submit_order(Request::new(SubmitOrderRequest {
            order_id: "buy-1".to_string(),
            user_id: "bob".to_string(),
            symbol: "BTC-USDT".to_string(),
            side: Side::Buy as i32,
            order_type: OrderType::Limit as i32,
            price: Some(65000.0),
            quantity: 1.5,
        }))
        .await
        .expect("submit limit buy")
        .into_inner();

    assert_eq!(buy_res.order_id, "buy-1");
    assert_eq!(buy_res.trades.len(), 1);
    let trade = &buy_res.trades[0];
    assert_eq!(trade.price, 65000.0);
    assert_eq!(trade.quantity, 1.5);
    assert_eq!(trade.symbol, "BTC-USDT");

    // Check Depth
    let depth_res = service
        .get_depth(Request::new(GetDepthRequest {
            symbol: "BTC-USDT".to_string(),
            limit: 10,
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(depth_res.bids.len(), 0);
    assert_eq!(depth_res.asks.len(), 1);
    assert_eq!(depth_res.asks[0].price, 65000.0);
    assert_eq!(depth_res.asks[0].quantity, 0.5);

    // Check Stats
    let stats_res = service
        .get_stats(Request::new(GetStatsRequest {
            symbol: "BTC-USDT".to_string(),
        }))
        .await
        .unwrap()
        .into_inner()
        .stats
        .unwrap();

    assert_eq!(stats_res.symbol, "BTC-USDT");
    assert_eq!(stats_res.total_trades_count, 1);
    assert_eq!(stats_res.total_volume_traded, 1.5);
    assert_eq!(stats_res.active_sell_volume, 0.5);
    assert_eq!(stats_res.active_buy_volume, 0.0);
    assert_eq!(stats_res.last_traded_price, Some(65000.0));

    // Check Trade History
    let history_res = service
        .get_trade_history(Request::new(GetTradeHistoryRequest {
            symbol: "BTC-USDT".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(history_res.trades.len(), 1);
    assert_eq!(history_res.trades[0].id, trade.id);

    // Cancel remaining sell order
    let cancel_res = service
        .cancel_order(Request::new(CancelOrderRequest {
            symbol: "BTC-USDT".to_string(),
            order_id: "sell-1".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();
    let cancelled_order = cancel_res.order.unwrap();
    assert_eq!(cancelled_order.id, "sell-1");
    assert_eq!(cancelled_order.status, OrderStatus::Cancelled as i32);
}

#[tokio::test]
async fn test_grpc_validation_errors() {
    let engine = Arc::new(ShardedEngine::new());
    let service = MatcherServiceImpl::new(engine);

    // Unknown symbol
    let res = service
        .submit_order(Request::new(SubmitOrderRequest {
            order_id: "o1".to_string(),
            user_id: "u1".to_string(),
            symbol: "UNKNOWN-PAIR".to_string(),
            side: Side::Buy as i32,
            order_type: OrderType::Limit as i32,
            price: Some(100.0),
            quantity: 1.0,
        }))
        .await;
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().code(), tonic::Code::Internal);

    // Register pair
    service
        .register_symbol(Request::new(RegisterSymbolRequest {
            symbol: "ETH-USDT".to_string(),
            listing_price: None,
        }))
        .await
        .unwrap();

    // Missing price for Limit Order
    let no_price_res = service
        .submit_order(Request::new(SubmitOrderRequest {
            order_id: "o2".to_string(),
            user_id: "u1".to_string(),
            symbol: "ETH-USDT".to_string(),
            side: Side::Buy as i32,
            order_type: OrderType::Limit as i32,
            price: None,
            quantity: 1.0,
        }))
        .await;
    assert!(no_price_res.is_err());

    // Zero quantity
    let zero_qty_res = service
        .submit_order(Request::new(SubmitOrderRequest {
            order_id: "o3".to_string(),
            user_id: "u1".to_string(),
            symbol: "ETH-USDT".to_string(),
            side: Side::Buy as i32,
            order_type: OrderType::Market as i32,
            price: None,
            quantity: 0.0,
        }))
        .await;
    assert!(zero_qty_res.is_err());
    assert_eq!(
        zero_qty_res.unwrap_err().code(),
        tonic::Code::InvalidArgument
    );
}
