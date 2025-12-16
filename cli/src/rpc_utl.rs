use std::time::Duration;
use solana_sdk::pubkey::Pubkey;
use anyhow::Result;

/// Get seed from Entropy API with optional samples parameter
/// 
/// # Arguments
/// * `var_address` - The VAR address (Pubkey)
/// * `samples` - Optional number of samples to request
/// 
/// # Returns
/// Returns `GetSeedResponse` containing seed, commit, address, end_slot, and samples
pub async fn get_entropy_seed(
    var_address: Pubkey,
    samples: Option<u64>,
) -> Result<entropy_types::response::GetSeedResponse> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    
    // Build URL with optional samples query parameter
    let mut url = format!("https://entropy-api.onrender.com/var/{}/seed", var_address);
    if let Some(samples_value) = samples {
        url = format!("{}?samples={}", url, samples_value);
    }
    
    tracing::debug!("Requesting entropy seed from: {}", url);
    
    let response = client
        .get(&url)
        .send()
        .await?;
    
    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "Entropy API returned error status {}: {}",
            status,
            error_text
        ));
    }
    
    let seed_response: entropy_types::response::GetSeedResponse = response
        .json()
        .await?;
    
    tracing::debug!(
        "Successfully retrieved entropy seed for VAR {}: end_slot={}, samples={}",
        var_address,
        seed_response.end_slot,
        seed_response.samples
    );
    
    Ok(seed_response)
}

/// Get seed from Entropy API with samples parameter (convenience function)
/// 
/// # Arguments
/// * `var_address` - The VAR address (Pubkey)
/// * `samples` - Number of samples to request
/// 
/// # Returns
/// Returns `GetSeedResponse` containing seed, commit, address, end_slot, and samples
pub async fn get_entropy_seed_with_samples(
    var_address: Pubkey,
    samples: u64,
) -> Result<entropy_types::response::GetSeedResponse> {
    get_entropy_seed(var_address, Some(samples)).await
}

/// Get seed from Entropy API without samples parameter (convenience function)
/// 
/// # Arguments
/// * `var_address` - The VAR address (Pubkey)
/// 
/// # Returns
/// Returns `GetSeedResponse` containing seed, commit, address, end_slot, and samples
pub async fn get_entropy_seed_default(
    var_address: Pubkey,
) -> Result<entropy_types::response::GetSeedResponse> {
    get_entropy_seed(var_address, None).await
}

/// Get slot hash from ORE BSM API
/// 
/// # Arguments
/// * `slot` - The slot number to get hash for
/// 
/// # Returns
/// Returns a 32-byte array representing the slot hash
/// 
/// # Example
/// ```
/// let slot_hash = get_slot_hash_from_api(387092935).await?;
/// ```
pub async fn get_slot_hash_from_api(slot: u64) -> Result<[u8; 32]> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    
    let url = format!("https://ore-bsm.onrender.com/slothash?slot={}", slot);
    
    tracing::debug!("Requesting slot hash from: {}", url);
    
    let response = client
        .get(&url)
        .send()
        .await?;
    
    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "ORE BSM API returned error status {}: {}",
            status,
            error_text
        ));
    }
    
    // Parse JSON array of u8 values
    let hash_array: Vec<u8> = response.json().await?;
    
    // Validate array length
    if hash_array.len() != 32 {
        return Err(anyhow::anyhow!(
            "Invalid slot hash length: expected 32 bytes, got {} bytes",
            hash_array.len()
        ));
    }
    
    // Convert Vec<u8> to [u8; 32]
    let slot_hash: [u8; 32] = hash_array
        .try_into()
        .map_err(|_| anyhow::anyhow!("Failed to convert Vec<u8> to [u8; 32]"))?;
    
    tracing::debug!("Successfully retrieved slot hash for slot {}", slot);
    
    Ok(slot_hash)
}
