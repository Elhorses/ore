use anyhow::Result;
use redis::AsyncCommands;
use std::sync::Arc;

use ore_api::consts::ONE_ORE;

/// Redis client wrapper for database operations
#[allow(dead_code)]
pub struct RedisClient {
    client: Arc<redis::Client>,
}

#[allow(dead_code)]
impl RedisClient {
    /// Create a new Redis client from connection string
    /// Example: "redis://127.0.0.1:6379"
    pub fn new(connection_string: &str) -> Result<Self> {
        let client = redis::Client::open(connection_string)?;
        Ok(Self {
            client: Arc::new(client),
        })
    }

    /// Get a connection from the client
    async fn get_connection(&self) -> Result<redis::aio::MultiplexedConnection> {
        let conn = self.client.get_multiplexed_async_connection().await?;
        Ok(conn)
    }

    /// Read a value from Redis
    pub async fn get<T: redis::FromRedisValue>(&self, key: &str) -> Result<Option<T>> {
        let mut conn = self.get_connection().await?;
        let value: Option<T> = conn.get(key).await?;
        Ok(value)
    }

    /// Write a value to Redis
    pub async fn set(&self, key: &str, value: &str) -> Result<()> {
        let mut conn = self.get_connection().await?;
        let _: () = redis::cmd("SET")
            .arg(key)
            .arg(value)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    /// Write a value to Redis with expiration (in seconds)
    pub async fn setex(&self, key: &str, value: &str, seconds: u64) -> Result<()> {
        let mut conn = self.get_connection().await?;
        let _: () = redis::cmd("SETEX")
            .arg(key)
            .arg(seconds)
            .arg(value)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    /// Update a value in Redis (set if not exists)
    pub async fn update(&self, key: &str, value: &str) -> Result<()> {
        let mut conn = self.get_connection().await?;
        let _: () = redis::cmd("SET")
            .arg(key)
            .arg(value)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }

    /// Delete a key from Redis
    pub async fn delete(&self, key: &str) -> Result<()> {
        let mut conn = self.get_connection().await?;
        let _: () = redis::cmd("DEL").arg(key).query_async(&mut conn).await?;
        Ok(())
    }

    /// Check if a key exists
    pub async fn exists(&self, key: &str) -> Result<bool> {
        let mut conn = self.get_connection().await?;
        let exists: bool = conn.exists(key).await?;
        Ok(exists)
    }

    /// Set a hash field
    pub async fn hset(&self, key: &str, field: &str, value: &str) -> Result<()> {
        let mut conn = self.get_connection().await?;
        let _: () = conn.hset(key, field, value).await?;
        Ok(())
    }

    /// Get a hash field
    pub async fn hget<T: redis::FromRedisValue>(
        &self,
        key: &str,
        field: &str,
    ) -> Result<Option<T>> {
        let mut conn = self.get_connection().await?;
        let value: Option<T> = conn.hget(key, field).await?;
        Ok(value)
    }

    /// Get all hash fields
    pub async fn hgetall<T: redis::FromRedisValue>(&self, key: &str) -> Result<Option<T>> {
        let mut conn = self.get_connection().await?;
        let value: Option<T> = conn.hgetall(key).await?;
        Ok(value)
    }

    /// Increment a hash field by a value
    pub async fn hincrby(&self, key: &str, field: &str, increment: i64) -> Result<i64> {
        let mut conn = self.get_connection().await?;
        let value: i64 = redis::cmd("HINCRBY")
            .arg(key)
            .arg(field)
            .arg(increment)
            .query_async(&mut conn)
            .await?;
        Ok(value)
    }

    /// Set multiple hash fields at once
    pub async fn hset_multiple(&self, key: &str, fields: &[(&str, &str)]) -> Result<()> {
        let mut conn = self.get_connection().await?;
        let _: () = conn.hset_multiple(key, fields).await?;
        Ok(())
    }

    /// Add a value to a set
    pub async fn sadd(&self, key: &str, member: &str) -> Result<()> {
        let mut conn = self.get_connection().await?;
        let _: () = conn.sadd(key, member).await?;
        Ok(())
    }

    /// Get all members of a set
    pub async fn smembers<T: redis::FromRedisValue>(&self, key: &str) -> Result<Option<T>> {
        let mut conn = self.get_connection().await?;
        let value: Option<T> = conn.smembers(key).await?;
        Ok(value)
    }

    /// Add a value to a sorted set with score
    pub async fn zadd(&self, key: &str, score: f64, member: &str) -> Result<()> {
        let mut conn = self.get_connection().await?;
        let _: () = conn.zadd(key, member, score).await?;
        Ok(())
    }

    /// Get range from a sorted set
    pub async fn zrange<T: redis::FromRedisValue>(
        &self,
        key: &str,
        start: isize,
        stop: isize,
    ) -> Result<Option<T>> {
        let mut conn = self.get_connection().await?;
        let value: Option<T> = conn.zrange(key, start, stop).await?;
        Ok(value)
    }

    /// Increment a value
    pub async fn incr(&self, key: &str) -> Result<i64> {
        let mut conn = self.get_connection().await?;
        let value: i64 = conn.incr(key, 1).await?;
        Ok(value)
    }

    /// Increment a value by a specific amount
    pub async fn incrby(&self, key: &str, increment: i64) -> Result<i64> {
        let mut conn = self.get_connection().await?;
        let value: i64 = conn.incr(key, increment).await?;
        Ok(value)
    }
}

/// Calculate the number of rounds until motherlode triggers based on current motherlode value
///
/// Motherlode mechanics:
/// - Each round adds 0.2 ORE (ONE_ORE / 5) to the motherlode pool
/// - Motherlode triggers every 625 rounds (1/625 probability)
/// - When triggered, the motherlode is paid out and reset to 0
///
/// # Arguments
/// * `motherlode` - Current motherlode value in grams (smallest unit, 10^11 grams = 1 ORE)
///
/// # Returns
/// Number of rounds until the next motherlode trigger (0-624)
#[allow(dead_code)]
pub fn calculate_motherlode_rounds_until_trigger(motherlode: u64) -> u64 {
    // Each round adds ONE_ORE / 5 = 0.2 ORE to motherlode
    // So the number of rounds since last trigger = motherlode / (ONE_ORE / 5) = motherlode * 5 / ONE_ORE
    // Since motherlode triggers every 625 rounds, rounds until next trigger = 625 - (rounds since last trigger % 625)

    if motherlode == 0 {
        return 625; // If motherlode is 0, it will trigger in 625 rounds
    }

    // Calculate rounds accumulated since last trigger
    // Using u128 to avoid overflow
    let rounds_accumulated = (motherlode as u128 * 5) / (ONE_ORE as u128);

    // Calculate rounds until next trigger (625 rounds per cycle)
    let rounds_until_trigger = 625 - (rounds_accumulated % 625);

    rounds_until_trigger as u64
}

/// Calculate the number of rounds since last motherlode trigger
///
/// # Arguments
/// * `motherlode` - Current motherlode value in grams
///
/// # Returns
/// Number of rounds since last trigger (0-624)
#[allow(dead_code)]
pub fn calculate_motherlode_rounds_since_trigger(motherlode: u64) -> u64 {
    if motherlode == 0 {
        return 0;
    }

    // Calculate rounds accumulated since last trigger
    let rounds_accumulated = (motherlode as u128 * 5) / (ONE_ORE as u128);

    // Get the remainder within the 625-round cycle
    (rounds_accumulated % 625) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_motherlode_calculations() {
        // Test with 0 motherlode (just reset)
        assert_eq!(calculate_motherlode_rounds_until_trigger(0), 625);
        assert_eq!(calculate_motherlode_rounds_since_trigger(0), 0);

        // Test with exactly 0.2 ORE (1 round worth)
        let one_round = ONE_ORE / 5;
        assert_eq!(calculate_motherlode_rounds_until_trigger(one_round), 624);
        assert_eq!(calculate_motherlode_rounds_since_trigger(one_round), 1);

        // Test with exactly 125 ORE (625 rounds worth, should trigger next round)
        let full_cycle = ONE_ORE * 125;
        assert_eq!(calculate_motherlode_rounds_until_trigger(full_cycle), 625);
        assert_eq!(calculate_motherlode_rounds_since_trigger(full_cycle), 0);

        // Test with 124.8 ORE (624 rounds worth, should trigger in 1 round)
        let almost_full_cycle = ONE_ORE * 124 + (ONE_ORE / 5) * 4;
        assert_eq!(
            calculate_motherlode_rounds_until_trigger(almost_full_cycle),
            1
        );
        assert_eq!(
            calculate_motherlode_rounds_since_trigger(almost_full_cycle),
            624
        );
    }
}
