use anyhow::Result;
use serde::{Deserialize, Serialize};
use solana_client::nonblocking::rpc_client::RpcClient;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::db::RedisClient;
use crate::get_round;

/// Winning tile statistics from a single tile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WinningTileStats {
    /// Tile number (1-25)
    pub tile: u8,
    /// Number of wins for this tile
    pub wins: u64,
    /// Win percentage
    pub percentage: f64,
}

/// Winning tiles API response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WinningTilesResponse {
    /// Total number of rounds
    #[serde(rename = "totalRounds")]
    pub total_rounds: u64,
    /// Statistics for each tile
    pub tiles: Vec<WinningTileStats>,
    /// last updated round id
    #[serde(rename = "latestRoundId")]
    pub latest_round_id: Option<u64>,
}

#[allow(dead_code)]
impl WinningTilesResponse {
    /// Fetch winning tiles data from the external API
    pub async fn fetch(api_url: &str) -> Result<Self> {
        let url = format!("{}/api/rounds/winning-tiles", api_url.trim_end_matches('/'));
        let response = reqwest::get(&url).await?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "API request failed with status: {}",
                response.status()
            ));
        }

        // Deserialize directly from JSON response
        let response_data: Self = response.json().await?;
        Ok(response_data)
    }

    /// Get statistics for a specific tile (1-25)
    pub fn get_tile_stats(&self, tile: u8) -> Option<&WinningTileStats> {
        self.tiles.iter().find(|t| t.tile == tile)
    }

    /// Get the tile with the most wins
    pub fn get_most_winning_tile(&self) -> Option<&WinningTileStats> {
        self.tiles.iter().max_by_key(|t| t.wins)
    }

    /// Get the tile with the least wins
    pub fn get_least_winning_tile(&self) -> Option<&WinningTileStats> {
        self.tiles.iter().min_by_key(|t| t.wins)
    }

    /// Get tiles sorted by win count (descending)
    pub fn get_tiles_sorted_by_wins(&self) -> Vec<&WinningTileStats> {
        let mut sorted: Vec<&WinningTileStats> = self.tiles.iter().collect();
        sorted.sort_by(|a, b| b.wins.cmp(&a.wins));
        sorted
    }

    /// Get tiles sorted by percentage (descending)
    pub fn get_tiles_sorted_by_percentage(&self) -> Vec<&WinningTileStats> {
        let mut sorted: Vec<&WinningTileStats> = self.tiles.iter().collect();
        sorted.sort_by(|a, b| {
            a.percentage
                .partial_cmp(&b.percentage)
                .unwrap_or(std::cmp::Ordering::Equal)
                .reverse()
        });
        sorted
    }
}

/// Cached winning tiles data with timestamp
#[derive(Clone)]
struct CachedWinningTiles {
    data: WinningTilesResponse,
    last_updated: Instant,
}

/// Thread-safe winning tiles cache manager
#[derive(Clone)]
pub struct WinningTilesCache {
    cache: Arc<RwLock<Option<CachedWinningTiles>>>,
    api_url: String,
    redis_client: Arc<RedisClient>,
    cache_ttl: Duration,
}

impl WinningTilesCache {
    /// Create a new cache manager
    pub fn new(
        api_url: Option<String>,
        cache_ttl_seconds: u64,
        redis_client: Arc<RedisClient>,
    ) -> Self {
        let api_url = api_url.unwrap_or_else(|| {
            std::env::var("WINNING_TILES_API_URL")
                .unwrap_or_else(|_| "https://refinorev2-production.up.railway.app".to_string())
        });

        Self {
            cache: Arc::new(RwLock::new(None)),
            api_url,
            redis_client,
            cache_ttl: Duration::from_secs(cache_ttl_seconds),
        }
    }

    /// Get winning tiles data (thread-safe, with caching)
    pub async fn get(&self, rpc: Arc<RpcClient>) -> Option<WinningTilesResponse> {
        // Check if cache is valid
        let cache_valid = {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.as_ref() {
                cached.last_updated.elapsed() < self.cache_ttl
            } else {
                false
            }
        };

        if cache_valid {
            // Return cached data
            let cache = self.cache.read().await;
            return cache.as_ref().map(|c| c.data.clone());
        }

        // Cache expired or doesn't exist, fetch new data
        self.refresh(rpc).await
    }

    /// Force refresh the cache
    pub async fn refresh(&self, rpc: Arc<RpcClient>) -> Option<WinningTilesResponse> {
        match WinningTilesResponse::fetch(&self.api_url).await {
            Ok(mut response) => {
                if let Ok(board) = crate::get_board(&rpc).await {
                    response.latest_round_id = Some(board.round_id);
                }
                let cached = CachedWinningTiles {
                    data: response.clone(),
                    last_updated: Instant::now(),
                };
                *self.cache.write().await = Some(cached);
                Some(response)
            }
            Err(e) => {
                eprintln!("Error fetching winning tiles: {}", e);
                // Try to load from redis
                if let Ok(Some(redis_json)) = self.redis_client.get::<String>("winning_tiles").await
                {
                    if let Ok(redis_data) =
                        serde_json::from_str::<WinningTilesResponse>(&redis_json)
                    {
                        return Some(redis_data);
                    }
                }

                // Return stale cache if available
                let cache = self.cache.read().await;
                cache.as_ref().map(|c| c.data.clone())
            }
        }
    }

    /// Start background refresh task
    pub fn start_background_refresh(&self, refresh_interval_seconds: u64, rpc: Arc<RpcClient>) {
        let cache = self.clone();
        let redis_client = self.redis_client.clone();
        let refresh_interval = Duration::from_secs(refresh_interval_seconds);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(refresh_interval);
            let mut pre_board = match crate::get_board(&rpc.clone()).await {
                Ok(board) => board,
                Err(e) => {
                    eprintln!("Error getting initial board: {}", e);
                    return;
                }
            };
            loop {
                interval.tick().await;
                let current_board = match crate::get_board(&rpc.clone()).await {
                    Ok(board) => board,
                    Err(e) => {
                        eprintln!("Error getting current board: {}", e);
                        continue;
                    }
                };
                if current_board.round_id > pre_board.round_id {
                    // Get current winning tiles from cache
                    let mut winning_tiles = match cache.cache.read().await.as_ref() {
                        Some(cached) => cached.data.clone(),
                        None => {
                            // Fallback: fetch from API if cache is empty
                            match get_winning_tiles_async(rpc.clone()).await {
                                Some(tiles) => tiles,
                                None => {
                                    eprintln!("Failed to get winning tiles, skipping update");
                                    continue;
                                }
                            }
                        }
                    };

                    for id in pre_board.round_id..current_board.round_id {
                        if let Ok(round) = get_round(&rpc.clone(), id).await {
                            // update the winning tiles data
                            if let Some(rng) = round.rng() {
                                let winning_square = round.winning_square(rng);
                                if winning_square < winning_tiles.tiles.len() {
                                    winning_tiles.tiles[winning_square].wins += 1;
                                    winning_tiles.total_rounds += 1;
                                }
                            }
                        }
                    }

                    // Update percentages for all tiles
                    for tile in winning_tiles.tiles.iter_mut() {
                        if winning_tiles.total_rounds > 0 {
                            tile.percentage =
                                tile.wins as f64 / winning_tiles.total_rounds as f64 * 100.0;
                        } else {
                            tile.percentage = 0.0;
                        }
                    }

                    winning_tiles.latest_round_id = Some(current_board.round_id);

                    // Update cache with modified winning tiles
                    *cache.cache.write().await = Some(CachedWinningTiles {
                        data: winning_tiles.clone(),
                        last_updated: Instant::now(),
                    });

                    // Update redis
                    if let Ok(json_str) = serde_json::to_string(&winning_tiles) {
                        if let Err(e) = redis_client
                            .set("winning_tiles", &json_str)
                            .await
                        {
                            eprintln!("Error updating redis: {}", e);
                        }
                    } else {
                        eprintln!("Error serializing winning tiles");
                    }

                    // Update pre_board for next iteration
                    pre_board = current_board;
                }
            }
        });
    }

    /// Get cache status
    #[allow(dead_code)]
    pub async fn cache_status(&self) -> Option<(Instant, Duration)> {
        let cache = self.cache.read().await;
        cache
            .as_ref()
            .map(|c| (c.last_updated, c.last_updated.elapsed()))
    }
}

/// Global winning tiles cache instance
static WINNING_TILES_CACHE: tokio::sync::OnceCell<WinningTilesCache> =
    tokio::sync::OnceCell::const_new();

/// Initialize the global winning tiles cache
pub async fn init_winning_tiles_cache(
    api_url: Option<String>,
    cache_ttl_seconds: u64,
    refresh_interval_seconds: u64,
    rpc: Arc<RpcClient>,
    redis_client: Arc<RedisClient>,
) -> Result<()> {
    let cache = WinningTilesCache::new(api_url, cache_ttl_seconds, redis_client);
    cache.start_background_refresh(refresh_interval_seconds, rpc.clone());
    // Do initial fetch
    cache.refresh(rpc).await;
    WINNING_TILES_CACHE
        .set(cache)
        .map_err(|_| anyhow::anyhow!("Cache already initialized"))?;
    Ok(())
}

/// Get winning tiles statistics (thread-safe, uses global cache)
/// Note: This function is deprecated, use get_winning_tiles_async instead
#[allow(dead_code)]
pub async fn get_winning_tiles() -> Option<WinningTilesResponse> {
    // get from global cache
    if let Some(cache) = WINNING_TILES_CACHE.get() {
        cache.cache.read().await.as_ref().map(|c| c.data.clone())
    } else {
        None
    }
}

/// Get winning tiles statistics (async version)
pub async fn get_winning_tiles_async(rpc: Arc<RpcClient>) -> Option<WinningTilesResponse> {
    if let Some(cache) = WINNING_TILES_CACHE.get() {
        cache.get(rpc).await
    } else {
        // Fallback to direct fetch if cache not initialized
        let api_url = std::env::var("WINNING_TILES_API_URL")
            .unwrap_or_else(|_| "https://refinorev2-production.up.railway.app".to_string());
        WinningTilesResponse::fetch(&api_url).await.ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fetch_winning_tiles() {
        let api_url = "https://refinorev2-production.up.railway.app";
        match WinningTilesResponse::fetch(api_url).await {
            Ok(response) => {
                println!("Total rounds: {}", response.total_rounds);
                println!("Number of tiles: {}", response.tiles.len());

                if let Some(most_winning) = response.get_most_winning_tile() {
                    println!(
                        "Most winning tile: {} with {} wins ({:.2}%)",
                        most_winning.tile, most_winning.wins, most_winning.percentage
                    );
                }

                if let Some(least_winning) = response.get_least_winning_tile() {
                    println!(
                        "Least winning tile: {} with {} wins ({:.2}%)",
                        least_winning.tile, least_winning.wins, least_winning.percentage
                    );
                }
            }
            Err(e) => {
                eprintln!("Error fetching winning tiles: {}", e);
            }
        }
    }
}
