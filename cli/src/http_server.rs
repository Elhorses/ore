use crate::db::RedisClient;
use crate::gmore::StateResponse;
use crate::winning_tile::WinningTilesResponse;
use axum::{extract::State, http::StatusCode, response::Json, routing::{get, post}, Router};
use ore_api::prelude::*;
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use std::sync::Arc;
use steel::Clock;
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

/// Shared state for HTTP server - stores current round and board data
#[derive(Clone)]
pub struct AppState {
    pub data: Arc<RwLock<RoundBoardData>>,
    pub redis_client: Option<Arc<RedisClient>>,
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
async fn get_combined_data(
    State(state): State<AppState>,
) -> Result<Json<RoundBoardData>, StatusCode> {
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
    match crate::winning_tile::get_winning_tiles().await {
        Some(tiles) => Ok(Json(tiles)),
        None => Err(StatusCode::SERVICE_UNAVAILABLE),
    }
}

/// Get gmore state from Redis
pub async fn get_gmore_state(
    State(state): State<AppState>,
) -> Result<Json<StateResponse>, StatusCode> {
    let redis_client = state
        .redis_client
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    match redis_client.get::<String>("gmore_state").await {
        Ok(Some(json_str)) => {
            match serde_json::from_str::<StateResponse>(&json_str) {
                Ok(state_response) => Ok(Json(state_response)),
                Err(e) => {
                    eprintln!("Error deserializing gmore_state from Redis: {:?}", e);
                    Err(StatusCode::INTERNAL_SERVER_ERROR)
                }
            }
        }
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(e) => {
            eprintln!("Error reading gmore_state from Redis: {:?}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Round history response item
#[derive(Debug, Serialize)]
pub struct RoundHistoryItem {
    pub round_id: String,
    #[serde(flatten)]
    pub data: crate::gmore::RoundResult,
}

/// Round history response
#[derive(Debug, Serialize)]
pub struct RoundHistoryResponse {
    pub rounds: Vec<RoundHistoryItem>,
}

/// Get round history from Redis
/// Returns the last 10 rounds including and before the specified round_id
pub async fn get_round_history(
    State(state): State<AppState>,
    axum::extract::Path(round_id): axum::extract::Path<u64>,
) -> Result<Json<RoundHistoryResponse>, StatusCode> {
    let redis_client = state
        .redis_client
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    // Calculate the range: from (round_id - 9) to round_id (inclusive)
    let start_round_id = if round_id >= 9 {
        round_id - 9
    } else {
        0
    };
    let end_round_id = round_id;

    // Query Redis for each round_id in the range
    let mut rounds = Vec::new();
    for id in start_round_id..=end_round_id {
        let key = id.to_string();
        match redis_client.get::<String>(&key).await {
            Ok(Some(json_str)) => {
                match serde_json::from_str::<crate::gmore::RoundResult>(&json_str) {
                    Ok(round_result) => {
                        rounds.push(RoundHistoryItem {
                            round_id: key.clone(),
                            data: round_result,
                        });
                    }
                    Err(e) => {
                        eprintln!("Error deserializing round {} from Redis: {:?}", id, e);
                        // Continue to next round instead of failing
                    }
                }
            }
            Ok(None) => {
                // Round not found in Redis, skip it
            }
            Err(e) => {
                eprintln!("Error reading round {} from Redis: {:?}", id, e);
                // Continue to next round instead of failing
            }
        }
    }

    // Sort by round_id descending (most recent first)
    rounds.sort_by(|a, b| {
        a.round_id
            .parse::<u64>()
            .unwrap_or(0)
            .cmp(&b.round_id.parse::<u64>().unwrap_or(0))
            .reverse()
    });

    Ok(Json(RoundHistoryResponse { rounds }))
}

/// Deploy request body
#[derive(Debug, Deserialize)]
pub struct DeployRequest {
    /// Array of 25 booleans indicating which squares to deploy to
    pub squares: [bool; 25],
    /// Authority address (miner address)
    pub authority: String,
    /// Round ID
    pub round_id: u64,
    /// Amount to deploy in lamports
    pub amount: u64,
}

/// Account metadata for instruction JSON (compatible with Solana Web3.js format)
#[derive(Debug, Serialize)]
struct AccountMetaJson {
    pubkey: String,
    #[serde(rename = "isSigner")]
    is_signer: bool,
    #[serde(rename = "isWritable")]
    is_writable: bool,
}

/// Deploy instruction response (compatible with Solana Web3.js TransactionInstruction format)
/// Frontend can directly use this to build and sign a transaction
#[derive(Debug, Serialize)]
pub struct DeployInstructionResponse {
    /// Program ID (base58 encoded)
    #[serde(rename = "programId")]
    program_id: String,
    /// Account metadata array
    accounts: Vec<AccountMetaJson>,
    /// Instruction data (base64 encoded)
    data: String,
}

/// Create deploy instruction
pub async fn create_deploy_instruction(
    Json(request): Json<DeployRequest>,
) -> Result<Json<DeployInstructionResponse>, (StatusCode, Json<serde_json::Value>)> {
    // Parse authority address
    let authority = Pubkey::from_str(&request.authority)
        .map_err(|e| {
            eprintln!("Invalid authority address: {} - Error: {:?}", request.authority, e);
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Invalid authority address",
                    "message": format!("The address '{}' is not a valid Solana address. Solana addresses should be base58 encoded (not 0x format). Example: 11111111111111111111111111111111", request.authority),
                    "received": request.authority
                }))
            )
        })?;

    // Validate squares array length
    if request.squares.len() != 25 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Invalid squares array length",
                "message": format!("Expected 25 squares, got {}", request.squares.len()),
                "received": request.squares.len()
            }))
        ));
    }

    // Get round_id and amount from request body
    let round_id = request.round_id;
    let amount = request.amount;

    // Create deploy instruction
    // Note: signer will be set by the client when building the transaction
    let deploy_ix = ore_api::sdk::deploy(
        authority, // signer (will be replaced by actual signer when building transaction)
        authority, // authority
        amount,
        round_id,
        request.squares,
    );

    // Convert instruction to JSON format
    let accounts: Vec<AccountMetaJson> = deploy_ix
        .accounts
        .iter()
        .map(|acc| AccountMetaJson {
            pubkey: acc.pubkey.to_string(),
            is_signer: acc.is_signer,
            is_writable: acc.is_writable,
        })
        .collect();

    // Encode instruction data as base64
    use base64::{Engine as _, engine::general_purpose};
    let data_base64 = general_purpose::STANDARD.encode(&deploy_ix.data);

    Ok(Json(DeployInstructionResponse {
        program_id: deploy_ix.program_id.to_string(),
        accounts,
        data: data_base64,
    }))
}

/// Create and start HTTP server
pub async fn start_http_server(
    state: AppState,
    port: u16,
    redis_client: Option<Arc<RedisClient>>,
) -> Result<(), anyhow::Error> {
    let mut app_state = state;
    app_state.redis_client = redis_client;

    let app = Router::new()
        .route("/health", get(health_check))
        .route("/api/round", get(get_round_data))
        .route("/api/board", get(get_board_data))
        .route("/api/data", get(get_combined_data))
        .route("/api/winning-tiles", get(get_winning_tiles))
        .route("/api/gmore-state", get(get_gmore_state))
        .route("/api/history/:round_id", get(get_round_history))
        .route("/api/deploy", post(create_deploy_instruction))
        // .route("/api/miner", get(get_miner_data))
        .layer(CorsLayer::permissive())
        .with_state(app_state.clone());

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    println!("HTTP server started on http://0.0.0.0:{}", port);
    println!("  GET /health - Health check");
    println!("  GET /api/round - Get current round data");
    println!("  GET /api/board - Get current board data");
    println!("  GET /api/data - Get combined round and board data");
    println!("  GET /api/winning-tiles - Get winning tiles statistics");
    println!("  GET /api/gmore-state - Get gmore state from Redis");
    println!("  GET /api/history/:round_id - Get round history (last 10 rounds)");
    println!("  POST /api/deploy - Create deploy instruction");
    // println!("  GET /api/miner - Get miner mining history round and board");

    axum::serve(listener, app).await?;
    Ok(())
}
