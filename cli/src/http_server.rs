use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::get,
    Router,
};
use ore_api::prelude::*;
use serde::Serialize;
use steel::Clock;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use crate::winning_tile::WinningTilesResponse;

/// Shared state for HTTP server - stores current round and board data
#[derive(Clone)]
pub struct AppState {
    pub data: Arc<RwLock<RoundBoardData>>,
}

/// Current round and board data
#[derive(Clone, Debug, Serialize)]
pub struct RoundBoardData {
    pub round: Round,
    pub board: Board,
    pub clock: Clock,
    pub treasury: Treasury,
    pub last_updated: u64, // Unix timestamp
}

impl RoundBoardData {
    pub fn new(round: Round, board: Board, clock: Clock, treasury: Treasury) -> Self {
        let unix_timestamp = clock.unix_timestamp as u64;
        Self {
            round,
            board,
            clock,
            treasury,
            last_updated: unix_timestamp,
        }
    }

    pub fn update(&mut self, round: Round, board: Board, clock: Clock, treasury: Treasury) {
        let unix_timestamp = clock.unix_timestamp as u64;
        self.round = round;
        self.board = board;
        self.clock = clock;
        self.treasury = treasury;
        self.last_updated = unix_timestamp;
    }
}

/// HTTP response for round data
#[derive(Serialize)]
pub struct RoundResponse {
    pub round_id: u64,
    pub deployed: [u64; 25],
    pub total_deployed: u64,
    pub expires_at: u64,
    pub current_slot: u64,
    pub last_updated: u64,
}

/// HTTP response for board data
#[derive(Serialize)]
pub struct BoardResponse {
    pub round_id: u64,
    pub start_slot: u64,
    pub end_slot: u64,
    pub current_slot: u64,
    pub last_updated: u64,
}

/// HTTP response for combined data
#[derive(Serialize)]
pub struct CombinedResponse {
    pub round: RoundResponse,
    pub board: BoardResponse,
}

/// Get current round data
async fn get_round_data(State(state): State<AppState>) -> Result<Json<RoundResponse>, StatusCode> {
    let data = state.data.read().await;
    Ok(Json(RoundResponse {
        round_id: data.round.id,
        deployed: data.round.deployed,
        total_deployed: data.round.total_deployed,
        expires_at: data.round.expires_at,
        current_slot: data.clock.slot,
        last_updated: data.last_updated,
    }))
}

/// Get current board data
async fn get_board_data(State(state): State<AppState>) -> Result<Json<BoardResponse>, StatusCode> {
    let data = state.data.read().await;
    Ok(Json(BoardResponse {
        round_id: data.board.round_id,
        start_slot: data.board.start_slot,
        end_slot: data.board.end_slot,
        current_slot: data.clock.slot,
        last_updated: data.last_updated,
    }))
}

/// Get combined round and board data
async fn get_combined_data(State(state): State<AppState>) -> Result<Json<RoundBoardData>, StatusCode> {
    let data = state.data.read().await;
    Ok(Json(data.clone()))
}

// async fn get_miner_data(State(state): State<AppState>) -> Result<Json<MinerData>, StatusCode> {
//     get_miner(&rpc, authority).await?;
//     Ok(Json(MinerData {
//         round_id: data.round.id,
//         deployed: data.round.deployed,
//         total_deployed: data.round.total_deployed,
//     }))
// }

/// Health check endpoint
pub async fn health_check() -> &'static str {
    "OK"
}

/// Get winning tiles statistics from external API (thread-safe, cached)
pub async fn get_winning_tiles() -> Result<Json<WinningTilesResponse>, StatusCode> {
    let winning_tiles = crate::winning_tile::get_winning_tiles().await.unwrap();
    Ok(Json(winning_tiles))
}

/// Create and start HTTP server
pub async fn start_http_server(state: AppState, port: u16) -> Result<(), anyhow::Error> {
    let app = Router::new()
        .route("/health", get(health_check))
        .route("/api/round", get(get_round_data))
        .route("/api/board", get(get_board_data))
        .route("/api/data", get(get_combined_data))
        .route("/api/winning-tiles", get(get_winning_tiles))
        // .route("/api/miner", get(get_miner_data))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    println!("HTTP server started on http://0.0.0.0:{}", port);
    println!("  GET /health - Health check");
    println!("  GET /api/round - Get current round data");
    println!("  GET /api/board - Get current board data");
    println!("  GET /api/data - Get combined round and board data");
    println!("  GET /api/winning-tiles - Get winning tiles statistics");
    // println!("  GET /api/miner - Get miner mining history round and board");
    
    axum::serve(listener, app).await?;
    Ok(())
}
