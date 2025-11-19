use std::sync::Arc;

use anyhow::Result;
use rand::Rng;
use serde::{Deserialize, Serialize};

/// Treasury information from the API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Treasury {
    #[serde(rename = "observedAt")]
    pub observed_at: String,
    #[serde(rename = "motherlodeFormatted")]
    pub motherlode_formatted: String,
}

/// Mining information for a round
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mining {
    #[serde(rename = "startSlot")]
    pub start_slot: String,
    #[serde(rename = "endSlot")]
    pub end_slot: String,
    #[serde(rename = "remainingSlots")]
    pub remaining_slots: String,
    pub status: String,
}

/// Totals for a round
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Totals {
    #[serde(rename = "deployedSol")]
    pub deployed_sol: String,
    #[serde(rename = "vaultedSol")]
    pub vaulted_sol: String,
    #[serde(rename = "winningsSol")]
    pub winnings_sol: String,
}

/// Per-square data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerSquare {
    pub counts: Vec<String>,
    #[serde(rename = "deployedSol")]
    pub deployed_sol: Vec<String>,
}

/// Round information from the API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Round {
    #[serde(rename = "observedAt")]
    pub observed_at: String,
    #[serde(rename = "roundId")]
    pub round_id: String,
    pub mining: Mining,
    #[serde(rename = "uniqueMiners")]
    pub unique_miners: String,
    pub totals: Totals,
    #[serde(rename = "perSquare")]
    pub per_square: PerSquare,
}

/// Price information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Price {
    #[serde(rename = "observedAt")]
    pub observed_at: String,
    #[serde(rename = "priceUsdRaw")]
    pub price_usd_raw: String,
}

/// Round result information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundResult {
    #[serde(rename = "roundId")]
    pub round_id: String,
    #[serde(rename = "resultAvailable")]
    pub result_available: bool,
    pub status: String,
    #[serde(rename = "slotHash")]
    pub slot_hash: String,
    pub rng: String,
    #[serde(rename = "winningSquareIndex")]
    pub winning_square_index: u8,
    #[serde(rename = "winningSquareLabel")]
    pub winning_square_label: String,
    #[serde(rename = "winningSquareDeployedRaw")]
    pub winning_square_deployed_raw: String,
    #[serde(rename = "winningSquareDeployedSol")]
    pub winning_square_deployed_sol: String,
    #[serde(rename = "winningSquareCount")]
    pub winning_square_count: String,
    #[serde(rename = "motherlodeHit")]
    pub motherlode_hit: bool,
    #[serde(rename = "motherlodeRaw")]
    pub motherlode_raw: String,
    #[serde(rename = "motherlodeFormatted")]
    pub motherlode_formatted: String,
    #[serde(rename = "topMiner")]
    pub top_miner: String,
    #[serde(rename = "topMinerRewardRaw")]
    pub top_miner_reward_raw: String,
    #[serde(rename = "topMinerRewardFormatted")]
    pub top_miner_reward_formatted: String,
    #[serde(rename = "numWinners")]
    pub num_winners: String,
    #[serde(rename = "totalDeployedRaw")]
    pub total_deployed_raw: String,
    #[serde(rename = "totalDeployedSol")]
    pub total_deployed_sol: String,
    #[serde(rename = "totalVaultedRaw")]
    pub total_vaulted_raw: String,
    #[serde(rename = "totalVaultedSol")]
    pub total_vaulted_sol: String,
    #[serde(rename = "totalWinningsRaw")]
    pub total_winnings_raw: String,
    #[serde(rename = "totalWinningsSol")]
    pub total_winnings_sol: String,
    #[serde(rename = "resultSource")]
    pub result_source: Option<String>,
    #[serde(rename = "resultPhase")]
    pub result_phase: Option<String>,
    #[serde(rename = "resultUpdatedAt")]
    pub result_updated_at: Option<String>,
}

/// Complete state response from the API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateResponse {
    pub treasury: Treasury,
    pub round: Round,
    #[serde(rename = "currentSlot")]
    pub current_slot: String,
    #[serde(rename = "orePrice")]
    pub ore_price: Price,
    #[serde(rename = "solPrice")]
    pub sol_price: Price,
    #[serde(rename = "roundResult")]
    pub round_result: RoundResult,
}

impl StateResponse {
    /// Fetch state data from the gmore API
    pub async fn fetch() -> Result<Self> {
        const API_URL: &str = "https://ore-api.gmore.fun/state";
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        let response = client.get(API_URL).send().await?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "API request failed with status: {}",
                response.status()
            ));
        }

        let state_data: Self = response.json().await?;
        Ok(state_data)
    }

    /// 定期获取state数据
    pub async fn fetch_periodically_gmore_state(redis_client: Arc<crate::db::RedisClient>) {
        // 随机生成300ms-2s秒的间隔
        let random_interval = rand::thread_rng().gen_range(500..2000);
        let interval = std::time::Duration::from_millis(random_interval);
        let mut interval = tokio::time::interval(interval);
        loop {
            interval.tick().await;
            match Self::fetch().await {
                Ok(state) => {
                    // 存储state数据到redis
                    match serde_json::to_string(&state) {
                        Ok(json_str) => {
                            if let Err(e) = redis_client.set("gmore_state", &json_str).await {
                                eprintln!("Error storing state to Redis: {:?}", e);
                            }
                        },
                        Err(e) => {
                            eprintln!("Error serializing state to JSON: {:?}", e);
                        }
                    }

                    // 存储上一轮的RoundResult数据到redis
                    match serde_json::to_string(&state.round_result) {
                        Ok(json_str) => {
                            if let Err(e) = redis_client.set(&state.round_result.round_id, &json_str).await {
                                eprintln!("Error storing round result to Redis: {:?}", e);
                            }
                        },
                        Err(e) => {
                            eprintln!("Error serializing round result to JSON: {:?}", e);
                        }
                    }
                },
                Err(e) => {
                    eprintln!("Error fetching state: {:?}", e);
                    continue;
                }
            };
        }
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fetch_state() {
        let state = StateResponse::fetch().await.unwrap();
        println!("{:?}", state);
    }
}