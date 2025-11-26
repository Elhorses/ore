use crate::db::RedisClient;
use crate::winning_tile::WinningTilesResponse;
use axum::{extract::{Path, State}, http::StatusCode, response::Json, routing::{get, post}, Router};
use ore_api::consts::SPLIT_ADDRESS;
use ore_api::prelude::*;
use serde::{Deserialize, Serialize};
use solana_account_decoder::UiAccountEncoding;
use solana_client::{
    nonblocking::rpc_client::RpcClient,
    rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    rpc_filter::{Memcmp, RpcFilterType},
};
use solana_sdk::pubkey::Pubkey;
use std::{str::FromStr, sync::Arc};
use steel::{AccountDeserialize, Clock, Discriminator};
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

/// Miner information for a specific round
#[derive(Serialize, serde::Deserialize)]
pub struct MinerInfo {
    pub address: String,
    pub authority: String,
    pub deployed: [u64; 25],
    pub cumulative: [u64; 25],
    pub deployed_sol: u64, // Total SOL deployed in this round (sum of all squares)
    pub rewards_sol: u64,  // SOL rewards earned in this round (round-specific)
    pub rewards_ore: u64,  // ORE rewards earned in this round (round-specific)
    pub refined_ore: u64,
    pub checkpoint_id: u64, // The round ID that this miner has checkpointed (checkpoint_id == round_id means checkpointed this round)
    pub has_checkpointed: bool, // Whether this miner has checkpointed this specific round
}

/// Detailed round data response
#[derive(Serialize, serde::Deserialize)]
pub struct RoundDetailResponse {
    pub round_id: u64,
    pub start_slot: u64,
    pub end_slot: u64,
    pub deployed: [u64; 25],
    pub count: [u64; 25],
    pub total_deployed: u64,
    pub total_vaulted: u64,
    pub total_winnings: u64,
    pub expires_at: u64,
    pub motherlode: u64,
    pub top_miner: String,
    pub top_miner_reward: u64,
    pub winning_square: Option<usize>,
    pub miners: Vec<MinerInfo>,
    pub miner_count: usize,
}

/// Detailed round data response
#[derive(Serialize, serde::Deserialize)]
pub struct HistoryRoundDetailResponse {
    pub round_id: u64,
    pub start_slot: u64,
    pub end_slot: u64,
    // pub deployed: [u64; 25],
    // pub count: [u64; 25],
    pub total_deployed: u64,
    pub total_vaulted: u64,
    pub total_winnings: u64,
    pub expires_at: u64,
    pub motherlode: u64,
    pub top_miner: String,
    pub top_miner_reward: u64,
    pub winning_square: Option<usize>,
    pub miner_count: usize,
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
    let winning_tiles = crate::winning_tile::get_winning_tiles().await.unwrap();
    Ok(Json(winning_tiles))
}

/// Calculate rewards for a miner in a specific round
fn calculate_round_rewards(round: &Round, miner: &Miner) -> (u64, u64) {
    let mut rewards_sol = 0;
    let mut rewards_ore = 0;

    // Get the RNG and winning square
    if let Some(rng) = round.rng() {
        let winning_square = round.winning_square(rng) as usize;

        // If the miner deployed to the winning square, calculate rewards
        if miner.deployed[winning_square] > 0 {
            // Calculate SOL rewards
            let original_deployment = miner.deployed[winning_square];
            let admin_fee = (original_deployment / 100).max(1);
            rewards_sol = original_deployment - admin_fee;
            if round.deployed[winning_square] > 0 {
                rewards_sol += ((round.total_winnings as u128
                    * miner.deployed[winning_square] as u128)
                    / round.deployed[winning_square] as u128) as u64;
            }

            // Calculate ORE rewards
            if round.top_miner == SPLIT_ADDRESS {
                // If round is split, split the reward evenly among all miners
                if round.deployed[winning_square] > 0 {
                    rewards_ore = ((round.top_miner_reward as u128
                        * miner.deployed[winning_square] as u128)
                        / round.deployed[winning_square] as u128)
                        as u64;
                }
            } else {
                // If round is not split, payout to the top miner
                let top_miner_sample = round.top_miner_sample(rng, winning_square);
                if top_miner_sample >= miner.cumulative[winning_square]
                    && top_miner_sample
                        < miner.cumulative[winning_square] + miner.deployed[winning_square]
                {
                    rewards_ore = round.top_miner_reward;
                }
            }

            // Calculate motherlode rewards
            // Note: round.deployed[winning_square] > 0 check is redundant since we're already inside
            // the miner.deployed[winning_square] > 0 check, but keeping for safety
            if round.motherlode > 0 && round.deployed[winning_square] > 0 {
                let motherload_rewards =
                    ((round.motherlode as u128 * miner.deployed[winning_square] as u128)
                        / round.deployed[winning_square] as u128) as u64;
                rewards_ore += motherload_rewards;
            }
        }
    } else {
        // Round has no slot hash, refund all SOL
        let refund_amount = miner.deployed.iter().sum::<u64>();
        rewards_sol = refund_amount;
    }

    (rewards_sol, rewards_ore)
}

/// Get detailed round data by round ID
pub async fn get_round_detail(
    Path(round_id): Path<u64>,
    State(state): State<AppState>,
) -> Result<Json<RoundDetailResponse>, StatusCode> {
    // Try to load from Redis 
    if let Some(redis_client) = &state.redis_client {
        let redis_key = format!("round:{}:detail", round_id);
        if let Ok(Some(cached_json)) = redis_client.get::<String>(&redis_key).await {
            if let Ok(cached_data) = serde_json::from_str::<RoundDetailResponse>(&cached_json) {
                eprintln!("Loaded round {} detail from Redis cache", round_id);
                return Ok(Json(cached_data));
            }
        }
    }

    // If not found in Redis, return error
    Err(StatusCode::NOT_FOUND)
}

/// Get round history - returns the last 10 rounds including the specified round_id
/// Returns rounds from (round_id - 9) to round_id (inclusive)
pub async fn get_round_history(
    Path(round_id): Path<u64>,
    State(state): State<AppState>,
) -> Result<Json<Vec<HistoryRoundDetailResponse>>, StatusCode> {
    let mut history = Vec::new();
    
    if let Some(redis_client) = &state.redis_client {
        // Query rounds from (round_id - 9) to round_id (inclusive), total 10 rounds
        let start_round = round_id.saturating_sub(9);
        
        tracing::debug!(
            "Querying round history for round_id {} (range: {} to {})",
            round_id,
            start_round,
            round_id
        );
        
        let mut found_count = 0;
        let mut not_found_count = 0;
        let mut error_count = 0;
        
        for id in start_round..=round_id {
            let redis_key = format!("round:{}:detail", id);
            match redis_client.get::<String>(&redis_key).await {
                Ok(Some(cached_json)) => {
                    match serde_json::from_str::<HistoryRoundDetailResponse>(&cached_json) {
                        Ok(cached_data) => {
                            history.push(cached_data);
                            found_count += 1;
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to parse round {} detail from Redis: {}",
                                id,
                                e
                            );
                            error_count += 1;
                        }
                    }
                }
                Ok(None) => {
                    // Key doesn't exist, which is fine
                    not_found_count += 1;
                    tracing::debug!("Round {} detail not found in Redis", id);
                }
                Err(e) => {
                    tracing::error!(
                        "Redis error when fetching round {} detail: {}",
                        id,
                        e
                    );
                    error_count += 1;
                }
            }
        }
        
        // Sort by round_id in descending order (newest first)
        history.sort_by(|a, b| b.round_id.cmp(&a.round_id));
        
        tracing::info!(
            "Loaded {} rounds from history for round_id {} (range: {} to {}). Found: {}, Not found: {}, Errors: {}",
            history.len(),
            round_id,
            start_round,
            round_id,
            found_count,
            not_found_count,
            error_count
        );
        
        // Return empty array if no data found (don't return error, as this is valid)
        Ok(Json(history))
    } else {
        tracing::error!("Redis client not available for round history query");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
}

/// Helper function to get round by ID
async fn get_round_by_id(rpc: &RpcClient, id: u64) -> Result<Round, anyhow::Error> {
    let round_pda = ore_api::state::round_pda(id);
    let account = rpc.get_account(&round_pda.0).await?;
    let round = Round::try_from_bytes(&account.data)?;
    Ok(*round)
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

/// Account metadata for instruction JSON (compatible with Solana Web3.js format)
#[derive(Debug, Serialize)]
struct AccountMetaJson {
    pubkey: String,
    #[serde(rename = "isSigner")]
    is_signer: bool,
    #[serde(rename = "isWritable")]
    is_writable: bool,
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

/// Helper function to get program accounts
async fn get_program_accounts<T>(
    client: &RpcClient,
    program_id: Pubkey,
    filters: Vec<RpcFilterType>,
) -> Result<Vec<(Pubkey, T)>, anyhow::Error>
where
    T: AccountDeserialize + Discriminator + Clone,
{
    let mut all_filters = vec![RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
        0,
        &T::discriminator().to_le_bytes(),
    ))];
    let filter_count = filters.len();
    all_filters.extend(filters);

    eprintln!(
        "Fetching program accounts with {} filters",
        all_filters.len()
    );

    let result = client
        .get_program_accounts_with_config(
            &program_id,
            RpcProgramAccountsConfig {
                filters: Some(all_filters),
                account_config: RpcAccountInfoConfig {
                    encoding: Some(UiAccountEncoding::Base64),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await;

    match result {
        Ok(accounts) => {
            let total_accounts = accounts.len();
            eprintln!("RPC returned {} raw accounts", total_accounts);

            let mut result = Vec::new();
            let mut parse_errors = 0;
            for (pubkey, account) in accounts {
                match T::try_from_bytes(&account.data) {
                    Ok(data) => {
                        result.push((pubkey, data.clone()));
                    }
                    Err(e) => {
                        parse_errors += 1;
                        // Log first few errors for debugging
                        if parse_errors <= 3 {
                            eprintln!("Failed to parse account {}: {:?}", pubkey, e);
                        }
                    }
                }
            }
            if parse_errors > 0 {
                eprintln!(
                    "Warning: {} accounts failed to parse out of {} total",
                    parse_errors, total_accounts
                );
            }
            eprintln!("Successfully parsed {} accounts", result.len());

            // Warn if we got a suspiciously small number of accounts
            if result.len() < 100 && filter_count == 0 {
                eprintln!(
                    "Warning: Only {} accounts returned, RPC may have a response size limit",
                    result.len()
                );
            }

            Ok(result)
        }
        Err(err) => {
            eprintln!("Error getting program accounts: {:?}", err);
            eprintln!("Error details: {}", err);
            Err(anyhow::anyhow!("Failed to get program accounts: {}", err))
        }
    }
}

/// Snapshot round data to Redis
/// This function should be called when a round ends or periodically to save round data
pub async fn snapshot_round_to_redis(
    rpc: Arc<RpcClient>,
    redis_client: Arc<RedisClient>,
    mut round: Round,
    board: Board,
) -> Result<(), anyhow::Error> {
    tracing::warn!("Creating snapshot for round {}...", round.id);

    // Get RNG and calculate winning square
    let rng = round
        .rng()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Failed to calculate RNG from slot_hash. slot_hash: {:?}",
                round.slot_hash
            )
        })?;
    let winning_square = round.winning_square(rng);

    // Get treasury to check motherlode
    let treasury_pda = ore_api::state::treasury_pda();
    let treasury_account = rpc.get_account(&treasury_pda.0).await?;
    let treasury = Treasury::try_from_bytes(&treasury_account.data)?;

    // Check if motherlode was triggered
    if round.did_hit_motherlode(rng) {
        round.motherlode = treasury.motherlode;
        tracing::info!("Motherlode triggered! Amount: {}", round.motherlode);
    }

    // Calculate admin fee
    let total_admin_fee = round.total_deployed / 100;

    // Calculate total_vaulted and total_winnings
    if round.deployed[winning_square] == 0 {
        // No one deployed on winning square, vault all deployed
        round.total_vaulted = round.total_deployed - total_admin_fee;
        round.total_winnings = 0;
    } else {
        // Calculate winnings (total deployed on non-winning squares)
        let winnings = round.calculate_total_winnings(winning_square);
        let winnings_admin_fee = winnings / 100; // 1% admin fee
        let winnings_after_fee = winnings - winnings_admin_fee;

        // 10% of winnings goes to vault
        let vault_amount = winnings_after_fee / 10;
        round.total_vaulted = vault_amount;
        round.total_winnings = winnings_after_fee - vault_amount;
    }

    // Set top_miner_reward (1 ORE, but for snapshot we use fixed value)
    // Note: In actual reset, this would be limited by MAX_SUPPLY, but for snapshot we use 1 ORE
    use ore_api::consts::ONE_ORE;
    round.top_miner_reward = ONE_ORE;

    // Determine if reward is split
    if round.is_split_reward(rng) {
        round.top_miner = SPLIT_ADDRESS;
        tracing::info!("Round reward is split among all miners");
    } else {
        // Find the top miner by sampling
        // Note: top_miner is set during checkpoint when a miner checkpoints and is the top miner
        // We need to find all miners who deployed on winning square
        
        // Get all miners - need to check both current round and checkpointed miners
        let all_miners = get_program_accounts::<Miner>(&rpc, ore_api::ID, vec![]).await?;
        
        // Collect miners who deployed on winning square and are currently in this round
        // (miners who checkpointed may have moved to next round, so we can't use their deployed data)
        let mut round_miners: Vec<(Pubkey, Miner, u64, u64)> = Vec::new(); // (pubkey, miner, cumulative, deployed)
        
        for (miner_pda, miner) in &all_miners {
            let is_current_round = miner.round_id == round.id;
            
            if is_current_round && miner.deployed[winning_square] > 0 {
                // Miner is still in this round, use their deployed data
                round_miners.push((
                    *miner_pda,
                    miner.clone(),
                    miner.cumulative[winning_square],
                    miner.deployed[winning_square],
                ));
            }
        }

        if !round_miners.is_empty() {
            // Sort miners by cumulative[winning_square] to ensure correct order
            round_miners.sort_by_key(|(_, _, cumulative, _)| *cumulative);
            
            let top_miner_sample = round.top_miner_sample(rng, winning_square);
            tracing::info!(
                "Finding top miner: sample={}, total_deployed={}, miners_count={}",
                top_miner_sample,
                round.deployed[winning_square],
                round_miners.len()
            );
            
            // Find the miner whose cumulative range includes the sample
            let mut found = false;
            for (_, miner, range_start, deployed_amount) in &round_miners {
                let range_end = range_start + deployed_amount;
                
                if top_miner_sample >= *range_start && top_miner_sample < range_end {
                    round.top_miner = miner.authority;
                    tracing::info!(
                        "Top miner found: {} (sample {} in range [{}, {}))",
                        miner.authority,
                        top_miner_sample,
                        range_start,
                        range_end
                    );
                    found = true;
                    break;
                }
            }
            
            // If no miner found in current round, check if round.top_miner was already set (from checkpoint)
            if !found {
                // Check if round.top_miner was set during checkpoint
                if round.top_miner != Pubkey::default() && round.top_miner != SPLIT_ADDRESS {
                    // Verify this miner checkpointed this round
                    let checkpointed = all_miners.iter().any(|(_, m)| {
                        m.checkpoint_id == round.id && m.authority == round.top_miner
                    });
                    if checkpointed {
                        tracing::info!(
                            "Top miner {} was set during checkpoint",
                            round.top_miner
                        );
                    } else {
                        tracing::warn!(
                            "round.top_miner {} is set but miner didn't checkpoint this round",
                            round.top_miner
                        );
                    }
                } else {
                    tracing::error!(
                        "No top miner found for sample {} (total deployed: {})",
                        top_miner_sample,
                        round.deployed[winning_square]
                    );
                }
            }
        } else {
            // Check if round.top_miner was set during checkpoint
            if round.top_miner != Pubkey::default() && round.top_miner != SPLIT_ADDRESS {
                tracing::info!(
                    "No miners in current round, but round.top_miner {} was set (from checkpoint)",
                    round.top_miner
                );
            } else {
                tracing::warn!("No miners found for winning square {}", winning_square);
            }
        }
    }

    tracing::info!(
        "Updated round {}: total_vaulted={}, total_winnings={}, top_miner={}, top_miner_reward={}, motherlode={}",
        round.id,
        round.total_vaulted,
        round.total_winnings,
        round.top_miner,
        round.top_miner_reward,
        round.motherlode
    );
    
    // Calculate winning square for response
    let winning_square = Some(winning_square);

    // Get all miners and filter for this round
    // IMPORTANT: We need to snapshot BEFORE miners move to next round
    // So we check both round_id and checkpoint_id
    let all_miners = get_program_accounts::<Miner>(&rpc, ore_api::ID, vec![]).await?;
    let miners: Vec<(Pubkey, Miner)> = all_miners
        .into_iter()
        .filter(|(_, miner)| {
            let is_current_round = miner.round_id == round.id;
            let has_checkpointed = miner.checkpoint_id == round.id;
            let winning_sq = winning_square.unwrap_or(0);
            
            // Include miners who:
            // 1. Are currently in this round and deployed on winning square, OR
            // 2. Checkpointed this round (they may have moved to next round, but we still want to show them)
            (is_current_round && miner.deployed[winning_sq] > 0) || has_checkpointed
        })
        .collect();

    tracing::warn!(
        "Found {} miners for round {} snapshot",
        miners.len(),
        round.id
    );

    // Convert to MinerInfo with rewards calculated
    let mut miner_infos: Vec<MinerInfo> = miners
        .iter()
        .map(|(miner_pda, miner)| {
            let is_current_round = miner.round_id == round.id;
            let has_checkpointed = miner.checkpoint_id == round.id;

            // IMPORTANT: When snapshotting, we need to calculate rewards while miner is still in this round
            // or has just checkpointed. Once they move to next round, deployed data is lost.
            let (round_rewards_sol, round_rewards_ore) = if is_current_round {
                // Miner is still in this round, calculate rewards from deployed data
                // This matches the logic in checkpoint.rs
                calculate_round_rewards(&round, miner)
            } else if has_checkpointed {
                // Miner has checkpointed but moved to next round
                // If they checkpointed, their deployed data should still be available for this round
                // Try to calculate rewards if deployed data is still valid (not reset to 0)
                let has_valid_deployed = miner.deployed.iter().any(|&d| d > 0);
                if has_valid_deployed {
                    // Deployed data still available, calculate rewards
                    calculate_round_rewards(&round, miner)
                } else {
                    // Deployed data has been reset, can't recalculate
                    // Note: In this case, rewards were already calculated and added to miner.rewards_sol/rewards_ore
                    // during checkpoint, but we can't show the round-specific breakdown
                    (0, 0)
                }
            } else {
                // Miner hasn't checkpointed and moved to next round (shouldn't happen)
                (0, 0)
            };

            let deployed_sol = if is_current_round {
                miner.deployed.iter().sum::<u64>()
            } else {
                0 // Miner has moved to next round, deployed data not available
            };

            MinerInfo {
                address: miner_pda.to_string(),
                authority: miner.authority.to_string(),
                deployed: if is_current_round {
                    miner.deployed
                } else {
                    [0; 25]
                },
                cumulative: if is_current_round {
                    miner.cumulative
                } else {
                    [0; 25]
                },
                deployed_sol,
                rewards_sol: round_rewards_sol,
                rewards_ore: round_rewards_ore,
                refined_ore: miner.refined_ore,
                checkpoint_id: if has_checkpointed { round.id } else { 0 }, // Show round_id if checkpointed, 0 otherwise
                has_checkpointed,
            }
        })
        .collect();

    // Sort by rewards_sol
    miner_infos.sort_by(|a, b| b.rewards_sol.cmp(&a.rewards_sol));

    // Create response
    let response = RoundDetailResponse {
        round_id: round.id,
        start_slot: board.start_slot,
        end_slot: board.end_slot,
        deployed: round.deployed,
        count: round.count,
        total_deployed: round.total_deployed,
        total_vaulted: round.total_vaulted,
        total_winnings: round.total_winnings,
        expires_at: round.expires_at,
        motherlode: round.motherlode,
        top_miner: round.top_miner.to_string(),
        top_miner_reward: round.top_miner_reward,
        winning_square,
        miners: miner_infos,
        miner_count: miners.len(),
    };

    // Save to Redis
    let redis_key = format!("round:{}:detail", round.id);
    let json = serde_json::to_string(&response)?;
    // Store permanently (no expiration) for historical data
    redis_client.set(&redis_key, &json).await?;

    tracing::info!("Successfully saved round {} snapshot to Redis", round.id);
    Ok(())
}

/// Create and start HTTP server
pub async fn start_http_server(state: AppState, port: u16) -> Result<(), anyhow::Error> {
    let app = Router::new()
        // check health
        .route("/health", get(health_check))
        // get current round data
        .route("/api/round", get(get_round_data))
        // get detailed round data by round ID
        .route("/api/round/:id", get(get_round_detail))
        // get round history (last 10 rounds including the specified round_id)
        .route("/api/round/history/:id", get(get_round_history))
        // get current board data
        .route("/api/board", get(get_board_data))
        // get combined round and board data
        .route("/api/data", get(get_combined_data))
        // get winning tiles statistics
        .route("/api/winning-tiles", get(get_winning_tiles))
        .route("/api/deploy", post(create_deploy_instruction))
        // .route("/api/miner", get(get_miner_data))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    println!("HTTP server started on http://0.0.0.0:{}", port);
    println!("  GET /health - Health check");
    println!("  GET /api/round - Get current round data");
    println!("  GET /api/round/:id - Get detailed round data by round ID");
    println!("  GET /api/round/history/:id - Get round history (last 10 rounds including the specified round_id)");
    println!("  GET /api/board - Get current board data");
    println!("  GET /api/data - Get combined round and board data");
    println!("  GET /api/winning-tiles - Get winning tiles statistics");
    println!("  POST /api/deploy - Create deploy instruction");
    // println!("  GET /api/miner - Get miner mining history round and board");

    axum::serve(listener, app).await?;
    Ok(())
}
