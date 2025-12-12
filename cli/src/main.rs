mod db;
mod http_server;
mod winning_tile;

use std::{collections::HashMap, str::FromStr, sync::Arc, time::Duration};

use entropy_api::prelude::*;
use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use http_server::{AppState, RoundBoardData};
use jup_swap::{
    quote::QuoteRequest,
    swap::SwapRequest,
    transaction_config::{DynamicSlippageSettings, TransactionConfig},
    JupiterSwapApiClient,
};
use ore_api::prelude::*;
use serde_json::{json, Value};
use solana_account_decoder::UiAccountEncoding;
use solana_client::{
    client_error::{reqwest::StatusCode, ClientErrorKind},
    nonblocking::rpc_client::RpcClient,
    rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig, RpcSendTransactionConfig},
    rpc_filter::{Memcmp, RpcFilterType},
};
use solana_sdk::{
    address_lookup_table::{state::AddressLookupTable, AddressLookupTableAccount},
    compute_budget::ComputeBudgetInstruction,
    message::v0::Message as SolanaMessage,
    message::VersionedMessage,
    native_token::{lamports_to_sol, LAMPORTS_PER_SOL},
    pubkey::Pubkey,
    rent::Rent,
    signature::{read_keypair_file, Signature, Signer},
    transaction::{Transaction, VersionedTransaction},
};
use solana_sdk::{keccak, pubkey};
use spl_associated_token_account::get_associated_token_address;
use spl_token::amount_to_ui_amount;
use steel::{AccountDeserialize, AccountMeta, Clock, Discriminator, Instruction};
use tokio::{
    net::TcpStream,
    sync::{mpsc, Mutex, RwLock},
};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

use crate::db::RedisClient;

use tracing::Level;

#[tokio::main]
async fn main() {
    // load log level from env
    let log_level = match std::env::var("LOG_LEVEL")
        .unwrap_or_else(|_| "info".to_string())
        .as_str()
    {
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };
    tracing_subscriber::fmt().with_max_level(log_level).init();

    // Read keypair from file
    let payer =
        read_keypair_file(&std::env::var("KEYPAIR").expect("Missing KEYPAIR env var")).unwrap();

    // Build transaction
    let rpc = RpcClient::new(std::env::var("RPC").expect("Missing RPC env var"));
    match std::env::var("COMMAND")
        .expect("Missing COMMAND env var")
        .as_str()
    {
        "automations" => {
            log_automations(&rpc).await.unwrap();
        }
        "clock" => {
            log_clock(&rpc).await.unwrap();
        }
        "claim" => {
            claim(&rpc, &payer).await.unwrap();
        }
        "board" => {
            log_board(&rpc).await.unwrap();
        }
        "config" => {
            log_config(&rpc).await.unwrap();
        }
        "buyback" => {
            buyback(&rpc, &payer).await.unwrap();
        }
        "reset" => {
            reset(&rpc, &payer).await.unwrap();
        }
        "treasury" => {
            log_treasury(&rpc).await.unwrap();
        }
        "miner" => {
            log_miner(&rpc, &payer).await.unwrap();
        }
        // "pool" => {
        //     log_meteora_pool(&rpc).await.unwrap();
        // }
        "deploy" => {
            deploy(&rpc, &payer).await.unwrap();
        }
        "stake" => {
            log_stake(&rpc, &payer).await.unwrap();
        }
        "deploy_all" => {
            deploy_all(&rpc, &payer).await.unwrap();
        }
        "round" => {
            log_round(&rpc).await.unwrap();
        }
        "set_admin" => {
            set_admin(&rpc, &payer).await.unwrap();
        }
        "set_fee_collector" => {
            set_fee_collector(&rpc, &payer).await.unwrap();
        }
        "ata" => {
            ata(&rpc, &payer).await.unwrap();
        }
        "checkpoint" => {
            checkpoint(&rpc, &payer).await.unwrap();
        }
        "checkpoint_all" => {
            checkpoint_all(&rpc, &payer).await.unwrap();
        }
        "close_all" => {
            close_all(&rpc, &payer).await.unwrap();
        }
        "participating_miners" => {
            participating_miners(&rpc).await.unwrap();
        }
        "new_var" => {
            new_var(&rpc, &payer).await.unwrap();
        }
        "set_admin_fee" => {
            set_admin_fee(&rpc, &payer).await.unwrap();
        }
        "set_swap_program" => {
            set_swap_program(&rpc, &payer).await.unwrap();
        }
        "set_var_address" => {
            set_var_address(&rpc, &payer).await.unwrap();
        }
        "keys" => {
            keys().await.unwrap();
        }
        "lut" => {
            lut(&rpc, &payer).await.unwrap();
        }
        "watch_deployed" => {
            let port = std::env::args()
                .nth(2)
                .and_then(|s| s.parse().ok())
                .unwrap_or(8080);
            // watch_deployed(Arc::new(rpc), port).await.unwrap();
            let rpc_urls = match std::env::var("RPC") {
                Ok(urls) => {
                    let urls: Vec<String> = urls
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if urls.is_empty() {
                        eprintln!("Error: RPC env var must contain at least one URL");
                        std::process::exit(1);
                    }
                    urls
                }
                Err(_) => {
                    eprintln!("Error: Missing RPC env var");
                    std::process::exit(1);
                }
            };
            match WatchDeployed::new(port, rpc_urls).await {
                Ok(mut wd) => {
                    if let Err(e) = wd.watch_deployed_scheduler().await {
                        tracing::error!("watch_deployed_scheduler error: {}", e);
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to initialize WatchDeployed: {}", e);
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        "sync_round" => {
            log_sync_round(&rpc).await.unwrap();
        }
        "liq" => {
            liq(&rpc, &payer).await.unwrap();
        }
        "automation" => {
            log_automation(&rpc).await.unwrap();
        }
        _ => panic!("Invalid command"),
    };
}

async fn liq(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
) -> Result<(), anyhow::Error> {
    let manager = pubkey!("DJqfQWB8tZE6fzqWa8okncDh7ciTuD8QQKp1ssNETWee");
    let wrap_ix = ore_api::sdk::wrap(payer.pubkey());
    let liq_ix = ore_api::sdk::liq(payer.pubkey(), manager);
    submit_transaction(rpc, payer, &[wrap_ix, liq_ix]).await?;
    Ok(())
}

async fn lut(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
) -> Result<(), anyhow::Error> {
    let recent_slot = rpc.get_slot().await? - 4;
    let (ix, lut_address) = solana_address_lookup_table_interface::instruction::create_lookup_table(
        payer.pubkey(),
        payer.pubkey(),
        recent_slot,
    );
    let ex_ix = solana_address_lookup_table_interface::instruction::extend_lookup_table(
        lut_address,
        payer.pubkey(),
        Some(payer.pubkey()),
        vec![
            pubkey!("HNWhK5f8RMWBqcA7mXJPaxdTPGrha3rrqUrri7HSKb3T"),
            pubkey!("2wQ7J46uwK3VyrmAYe5E8KhCjTg8CTaFimh1ty2huuyY"),
            pubkey!("DJqfQWB8tZE6fzqWa8okncDh7ciTuD8QQKp1ssNETWee"),
            pubkey!("HLaJ3RiyoaxQzwJQbU2Gc5RTZtx8HKAMJgkf57qdgpFJ"),
            pubkey!("8yS5zJTZa1Q1zQ1jsEAUnjAyMZfsNwvrgbDQp1ky2dr"),
            pubkey!("7qBS6huLjjGyrnMMBNXpLZA73yiGc6ao9znj7f9RpF1L"),
            pubkey!("3Mt1bpU3fnSXyPEm66HKKXyQTpLWrwYziPLqwTqK4ZT7"),
            pubkey!("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo"),
            pubkey!("oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp"),
            pubkey!("So11111111111111111111111111111111111111112"),
            pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"),
            pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr"),
            pubkey!("D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6"),
            pubkey!("8kqLv9cBUDCYEKCL3Dj2MkeXX3tdCqT8KZ3gpYp8BnGP"),
            pubkey!("H38TVzkjAiAhBZR5SksbW8XDXP3N1ez4Tuna7uAW1Tsw"),
            pubkey!("11111111111111111111111111111111"),
            pubkey!("SysvarRent111111111111111111111111111111111"),
        ],
    );
    let ix_1 = Instruction {
        program_id: ix.program_id,
        accounts: ix
            .accounts
            .iter()
            .map(|a| AccountMeta::new(a.pubkey, a.is_signer))
            .collect(),
        data: ix.data,
    };
    let ix_2 = Instruction {
        program_id: ex_ix.program_id,
        accounts: ex_ix
            .accounts
            .iter()
            .map(|a| AccountMeta::new(a.pubkey, a.is_signer))
            .collect(),
        data: ex_ix.data,
    };
    submit_transaction(rpc, payer, &[ix_1, ix_2]).await?;
    println!("LUT address: {}", lut_address);
    Ok(())
}

async fn set_admin_fee(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
) -> Result<(), anyhow::Error> {
    let admin_fee = std::env::var("ADMIN_FEE").expect("Missing ADMIN_FEE env var");
    let admin_fee = u64::from_str(&admin_fee).expect("Invalid ADMIN_FEE");
    let ix = ore_api::sdk::set_admin_fee(payer.pubkey(), admin_fee);
    submit_transaction(rpc, payer, &[ix]).await?;
    Ok(())
}

async fn set_var_address(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
) -> Result<(), anyhow::Error> {
    let new_var_address = std::env::var("VAR").expect("Missing VAR env var");
    let new_var_address = Pubkey::from_str(&new_var_address).expect("Invalid VAR");
    let ix = ore_api::sdk::set_var_address(payer.pubkey(), new_var_address);
    submit_transaction(rpc, payer, &[ix]).await?;
    Ok(())
}

async fn new_var(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
) -> Result<(), anyhow::Error> {
    let provider = std::env::var("PROVIDER").expect("Missing PROVIDER env var");
    let provider = Pubkey::from_str(&provider).expect("Invalid PROVIDER");
    let commit = std::env::var("COMMIT").expect("Missing COMMIT env var");
    let commit = keccak::Hash::from_str(&commit).expect("Invalid COMMIT");
    let samples = std::env::var("SAMPLES").expect("Missing SAMPLES env var");
    let samples = u64::from_str(&samples).expect("Invalid SAMPLES");
    let board_address = board_pda().0;
    let var_address = entropy_api::state::var_pda(board_address, 0).0;
    println!("Var address: {}", var_address);
    let ix = ore_api::sdk::new_var(payer.pubkey(), provider, 0, commit.to_bytes(), samples);
    submit_transaction(rpc, payer, &[ix]).await?;
    Ok(())
}

async fn participating_miners(rpc: &RpcClient) -> Result<(), anyhow::Error> {
    let round_id = std::env::var("ID").expect("Missing ID env var");
    let round_id = u64::from_str(&round_id).expect("Invalid ID");
    let miners = get_miners_participating(rpc, round_id).await?;
    for (i, (_address, miner)) in miners.iter().enumerate() {
        println!("{}: {}", i, miner.authority);
    }
    Ok(())
}

async fn log_stake(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
) -> Result<(), anyhow::Error> {
    let authority = std::env::var("AUTHORITY").unwrap_or(payer.pubkey().to_string());
    let authority = Pubkey::from_str(&authority).expect("Invalid AUTHORITY");
    let staker_address = ore_api::state::stake_pda(authority).0;
    let stake = get_stake(rpc, authority).await?;
    println!("Stake");
    println!("  address: {}", staker_address);
    println!("  authority: {}", authority);
    println!(
        "  balance: {} ORE",
        amount_to_ui_amount(stake.balance, TOKEN_DECIMALS)
    );
    println!("  buffer_a: {}", stake.buffer_a);
    println!("  buffer_b: {}", stake.buffer_b);
    println!("  buffer_c: {}", stake.buffer_c);
    println!("  buffer_d: {}", stake.buffer_d);
    println!("  buffer_e: {}", stake.buffer_e);
    println!("  last_claim_at: {}", stake.last_claim_at);
    println!("  last_deposit_at: {}", stake.last_deposit_at);
    println!("  last_withdraw_at: {}", stake.last_withdraw_at);
    println!(
        "  rewards_factor: {}",
        stake.rewards_factor.to_i80f48().to_string()
    );
    println!(
        "  rewards: {} ORE",
        amount_to_ui_amount(stake.rewards, TOKEN_DECIMALS)
    );
    println!(
        "  lifetime_rewards: {} ORE",
        amount_to_ui_amount(stake.lifetime_rewards, TOKEN_DECIMALS)
    );
    println!("  buffer_f: {}", stake.buffer_f);

    Ok(())
}

async fn ata(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
) -> Result<(), anyhow::Error> {
    let user = pubkey!("FgZFnb3bi7QexKCdXWPwWy91eocUD7JCFySHb83vLoPD");
    let token = pubkey!("8H8rPiWW4iTFCfEkSnf7jpqeNpFfvdH9gLouAL3Fe2Zx");
    let ata = get_associated_token_address(&user, &token);
    let ix = spl_associated_token_account::instruction::create_associated_token_account(
        &payer.pubkey(),
        &user,
        &token,
        &spl_token::ID,
    );
    submit_transaction(rpc, payer, &[ix]).await?;
    let account = rpc.get_account(&ata).await?;
    println!("ATA: {}", ata);
    println!("Account: {:?}", account);
    Ok(())
}

async fn keys() -> Result<(), anyhow::Error> {
    let treasury_address = ore_api::state::treasury_pda().0;
    let config_address = ore_api::state::config_pda().0;
    let board_address = ore_api::state::board_pda().0;
    let address = pubkey!("pqspJ298ryBjazPAr95J9sULCVpZe3HbZTWkbC1zrkS");
    let miner_address = ore_api::state::miner_pda(address).0;
    let round = round_pda(31460).0;
    println!("Round: {}", round);
    println!("Treasury: {}", treasury_address);
    println!("Config: {}", config_address);
    println!("Board: {}", board_address);
    println!("Miner: {}", miner_address);
    Ok(())
}

async fn claim(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
) -> Result<(), anyhow::Error> {
    let ix_sol = ore_api::sdk::claim_sol(payer.pubkey());
    let ix_ore = ore_api::sdk::claim_ore(payer.pubkey());
    submit_transaction(rpc, payer, &[ix_sol, ix_ore]).await?;
    Ok(())
}

async fn buyback(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
) -> Result<(), anyhow::Error> {
    // Get swap amount.
    let treasury = get_treasury(rpc).await?;
    let amount = treasury.balance.min(10 * LAMPORTS_PER_SOL);

    // Build quote request.
    const INPUT_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");
    const OUTPUT_MINT: Pubkey = pubkey!("oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp");
    let api_base_url =
        std::env::var("API_BASE_URL").unwrap_or("https://lite-api.jup.ag/swap/v1".into());
    let jupiter_swap_api_client = JupiterSwapApiClient::new(api_base_url);
    let quote_request = QuoteRequest {
        amount,
        input_mint: INPUT_MINT,
        output_mint: OUTPUT_MINT,
        max_accounts: Some(55),
        ..QuoteRequest::default()
    };

    // GET /quote
    let quote_response = match jupiter_swap_api_client.quote(&quote_request).await {
        Ok(quote_response) => quote_response,
        Err(e) => {
            println!("quote failed: {e:#?}");
            return Err(anyhow::anyhow!("quote failed: {e:#?}"));
        }
    };

    // GET /swap/instructions
    let treasury_address = ore_api::state::treasury_pda().0;
    let response = jupiter_swap_api_client
        .swap_instructions(&SwapRequest {
            user_public_key: treasury_address,
            quote_response,
            config: TransactionConfig {
                skip_user_accounts_rpc_calls: true,
                wrap_and_unwrap_sol: false,
                dynamic_compute_unit_limit: true,
                dynamic_slippage: Some(DynamicSlippageSettings {
                    min_bps: Some(50),
                    max_bps: Some(1000),
                }),
                ..TransactionConfig::default()
            },
        })
        .await
        .unwrap();

    let address_lookup_table_accounts =
        get_address_lookup_table_accounts(rpc, response.address_lookup_table_addresses)
            .await
            .unwrap();

    // Build transaction.
    let wrap_ix = ore_api::sdk::wrap(payer.pubkey());
    let buyback_ix = ore_api::sdk::buyback(
        payer.pubkey(),
        &response.swap_instruction.accounts,
        &response.swap_instruction.data,
    );
    simulate_transaction_with_address_lookup_tables(
        rpc,
        payer,
        &[wrap_ix, buyback_ix],
        address_lookup_table_accounts,
    )
    .await;

    Ok(())
}

#[allow(dead_code)]
pub async fn get_address_lookup_table_accounts(
    rpc_client: &RpcClient,
    addresses: Vec<Pubkey>,
) -> Result<Vec<AddressLookupTableAccount>, anyhow::Error> {
    let mut accounts = Vec::new();
    for key in addresses {
        if let Ok(account) = rpc_client.get_account(&key).await {
            if let Ok(address_lookup_table_account) = AddressLookupTable::deserialize(&account.data)
            {
                accounts.push(AddressLookupTableAccount {
                    key,
                    addresses: address_lookup_table_account.addresses.to_vec(),
                });
            }
        }
    }
    Ok(accounts)
}

pub const ORE_VAR_ADDRESS: Pubkey = pubkey!("BWCaDY96Xe4WkFq1M7UiCCRcChsJ3p51L5KrGzhxgm2E");

async fn reset(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
) -> Result<(), anyhow::Error> {
    let board = get_board(rpc).await?;
    let var = get_var(rpc, ORE_VAR_ADDRESS).await?;

    println!("Var: {:?}", var);

    let client = reqwest::Client::new();
    let url = format!("https://entropy-api.onrender.com/var/{ORE_VAR_ADDRESS}/seed");
    let response = client
        .get(url)
        .send()
        .await?
        .json::<entropy_types::response::GetSeedResponse>()
        .await?;
    println!("Entropy seed: {:?}", response);

    let config = get_config(rpc).await?;
    let sample_ix = entropy_api::sdk::sample(payer.pubkey(), ORE_VAR_ADDRESS);
    let reveal_ix = entropy_api::sdk::reveal(payer.pubkey(), ORE_VAR_ADDRESS, response.seed);
    let reset_ix = ore_api::sdk::reset(
        payer.pubkey(),
        config.fee_collector,
        board.round_id,
        Pubkey::default(),
    );
    let sig = submit_transaction(rpc, payer, &[sample_ix, reveal_ix, reset_ix]).await?;
    println!("Reset: {}", sig);

    // let slot_hashes = get_slot_hashes(rpc).await?;
    // if let Some(slot_hash) = slot_hashes.get(&board.end_slot) {
    //     let id = get_winning_square(&slot_hash.to_bytes());
    //     // let square = get_square(rpc).await?;
    //     println!("Winning square: {}", id);
    //     // println!("Miners: {:?}", square.miners);
    //     // miners = square.miners[id as usize].to_vec();
    // };

    // let reset_ix = ore_api::sdk::reset(
    //     payer.pubkey(),
    //     config.fee_collector,
    //     board.round_id,
    //     Pubkey::default(),
    // );
    // // simulate_transaction(rpc, payer, &[reset_ix]).await;
    // submit_transaction(rpc, payer, &[reset_ix]).await?;
    Ok(())
}

async fn deploy(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
) -> Result<(), anyhow::Error> {
    let amount = std::env::var("AMOUNT").expect("Missing AMOUNT env var");
    let amount = u64::from_str(&amount).expect("Invalid AMOUNT");
    let square_id = std::env::var("SQUARE").expect("Missing SQUARE env var");
    let square_id = u64::from_str(&square_id).expect("Invalid SQUARE");
    // let board = get_board(rpc).await?;

    let round_id = std::env::var("ROUND").expect("Missing ROUND env var");
    let round_id = u64::from_str(&round_id).expect("Invalid ROUND");

    // Check board time range before deploying
    // let mut instructions = vec![];
    // if let Some(check_ins) = deploy_check(rpc, payer, &board).await? {
    //     instructions.extend(check_ins);
    // }
    let authority = payer.pubkey();
    let mut instructions = vec![];

    if let Ok(miner) = get_miner(rpc, authority).await {
        // If miner is on a different round and hasn't checkpointed, add checkpoint instruction
        let checkpoint_ix = ore_api::sdk::checkpoint(payer.pubkey(), authority, miner.round_id);
        instructions.push(checkpoint_ix);
    }

    // Add deploy instruction
    let mut squares = [false; 25];
    squares[square_id as usize] = true;
    let deploy_ix = ore_api::sdk::deploy(payer.pubkey(), payer.pubkey(), amount, round_id, squares);
    instructions.push(deploy_ix);

    // Submit all instructions in a single transaction
    if instructions.len() > 1 {
        println!("Submitting checkpoint and deploy in a single transaction...");
    }
    // std::process::exit(0);
    submit_transaction(rpc, payer, &instructions).await?;
    Ok(())
}

async fn deploy_all(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
) -> Result<(), anyhow::Error> {
    let amount = std::env::var("AMOUNT").expect("Missing AMOUNT env var");
    let amount = u64::from_str(&amount).expect("Invalid AMOUNT");
    let board = get_board(rpc).await?;

    // Check board time range before deploying
    let mut instructions = vec![];
    if let Some(check_ins) = deploy_check(rpc, payer, &board).await? {
        instructions.extend(check_ins);
    }

    // Add deploy instruction
    let squares = [true; 25];
    let deploy_ix = ore_api::sdk::deploy(
        payer.pubkey(),
        payer.pubkey(),
        amount,
        board.round_id,
        squares,
    );
    instructions.push(deploy_ix);

    // Submit all instructions in a single transaction
    if instructions.len() > 1 {
        println!("Submitting checkpoint and deploy_all in a single transaction...");
    }
    // 加一个收手续费的 trasfer %1
    submit_transaction(rpc, payer, &instructions).await?;
    Ok(())
}

async fn deploy_check(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
    board: &Board,
) -> Result<Option<Vec<Instruction>>, anyhow::Error> {
    // Check board time range before deploying
    let clock = get_clock(rpc).await?;
    println!(
        "Board state: round_id={}, start_slot={}, end_slot={}",
        board.round_id, board.start_slot, board.end_slot
    );
    println!("Current slot: {}", clock.slot);

    // Check if board is in valid time range
    if board.end_slot != u64::MAX && (clock.slot < board.start_slot || clock.slot >= board.end_slot)
    {
        if clock.slot < board.start_slot {
            return Err(anyhow::anyhow!(
                "Board round has not started yet. Current slot: {}, Start slot: {}",
                clock.slot,
                board.start_slot
            ));
        } else {
            return Err(anyhow::anyhow!(
                "Board round has expired. Current slot: {}, End slot: {}",
                clock.slot,
                board.end_slot
            ));
        }
    }

    // Check if miner needs checkpoint before deploying to new round
    let authority = payer.pubkey();
    let mut instructions = vec![];

    if let Ok(miner) = get_miner(rpc, authority).await {
        // If miner is on a different round and hasn't checkpointed, add checkpoint instruction
        if miner.round_id != board.round_id && miner.checkpoint_id != miner.round_id {
            println!("Miner needs checkpoint before deploying to new round. Adding checkpoint instruction for round {}...", miner.round_id);
            let checkpoint_ix = ore_api::sdk::checkpoint(payer.pubkey(), authority, miner.round_id);
            instructions.push(checkpoint_ix);
        }
    }

    // Verify round account exists before deploying
    let round_pda = ore_api::state::round_pda(board.round_id);
    match rpc.get_account(&round_pda.0).await {
        Ok(account) => {
            // Try to parse as Round to verify data format
            match Round::try_from_bytes(&account.data) {
                Ok(round) => {
                    if round.id != board.round_id {
                        return Err(anyhow::anyhow!(
                            "Round account ID mismatch: expected {}, got {}",
                            board.round_id,
                            round.id
                        ));
                    }
                    println!("Round account verified: round_id={}", round.id);
                }
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "Round account data format invalid: {}. Round account may need to be reset.",
                        e
                    ));
                }
            }
        }
        Err(_) => {
            return Err(anyhow::anyhow!(
                "Round account for round {} does not exist. The round may need to be reset first.",
                board.round_id
            ));
        }
    }
    Ok(Some(instructions))
}

async fn set_admin(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
) -> Result<(), anyhow::Error> {
    let ix = ore_api::sdk::set_admin(payer.pubkey(), payer.pubkey());
    submit_transaction(rpc, payer, &[ix]).await?;
    Ok(())
}

async fn set_swap_program(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
) -> Result<(), anyhow::Error> {
    let swap_program = std::env::var("SWAP_PROGRAM").expect("Missing SWAP_PROGRAM env var");
    let swap_program = Pubkey::from_str(&swap_program).expect("Invalid SWAP_PROGRAM");
    let ix = ore_api::sdk::set_swap_program(payer.pubkey(), swap_program);
    submit_transaction(rpc, payer, &[ix]).await?;
    Ok(())
}

async fn set_fee_collector(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
) -> Result<(), anyhow::Error> {
    let fee_collector = std::env::var("FEE_COLLECTOR").expect("Missing FEE_COLLECTOR env var");
    let fee_collector = Pubkey::from_str(&fee_collector).expect("Invalid FEE_COLLECTOR");
    let ix = ore_api::sdk::set_fee_collector(payer.pubkey(), fee_collector);
    submit_transaction(rpc, payer, &[ix]).await?;
    Ok(())
}

async fn checkpoint(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
) -> Result<(), anyhow::Error> {
    let authority = std::env::var("AUTHORITY").unwrap_or(payer.pubkey().to_string());
    let authority = Pubkey::from_str(&authority).expect("Invalid AUTHORITY");
    let miner = get_miner(rpc, authority).await?;
    let ix = ore_api::sdk::checkpoint(payer.pubkey(), authority, miner.round_id);
    submit_transaction(rpc, payer, &[ix]).await?;
    Ok(())
}

async fn checkpoint_all(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
) -> Result<(), anyhow::Error> {
    let clock = get_clock(rpc).await?;
    let miners = get_miners(rpc).await?;
    let mut expiry_slots = HashMap::new();
    let mut ixs = vec![];
    for (i, (_address, miner)) in miners.iter().enumerate() {
        if miner.checkpoint_id < miner.round_id {
            // Log the expiry slot for the round.
            if !expiry_slots.contains_key(&miner.round_id) {
                if let Ok(round) = get_round(rpc, miner.round_id).await {
                    expiry_slots.insert(miner.round_id, round.expires_at);
                }
            }

            // Get the expiry slot for the round.
            let Some(expires_at) = expiry_slots.get(&miner.round_id) else {
                continue;
            };

            // If we are in fee collection period, checkpoint the miner.
            if clock.slot >= expires_at - TWELVE_HOURS_SLOTS {
                println!(
                    "[{}/{}] Checkpoint miner: {} ({} s)",
                    i + 1,
                    miners.len(),
                    miner.authority,
                    (expires_at - clock.slot) as f64 * 0.4
                );
                ixs.push(ore_api::sdk::checkpoint(
                    payer.pubkey(),
                    miner.authority,
                    miner.round_id,
                ));
            }
        }
    }

    // Batch and submit the instructions.
    while !ixs.is_empty() {
        let batch = ixs
            .drain(..std::cmp::min(10, ixs.len()))
            .collect::<Vec<Instruction>>();
        submit_transaction(rpc, payer, &batch).await?;
    }

    Ok(())
}

async fn close_all(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
) -> Result<(), anyhow::Error> {
    let rounds = get_rounds(rpc).await?;
    let mut ixs = vec![];
    let clock = get_clock(rpc).await?;
    for (_i, (_address, round)) in rounds.iter().enumerate() {
        if clock.slot >= round.expires_at {
            ixs.push(ore_api::sdk::close(
                payer.pubkey(),
                round.id,
                round.rent_payer,
            ));
        }
    }

    // Batch and submit the instructions.
    while !ixs.is_empty() {
        let batch = ixs
            .drain(..std::cmp::min(12, ixs.len()))
            .collect::<Vec<Instruction>>();
        // simulate_transaction(rpc, payer, &batch).await;
        submit_transaction(rpc, payer, &batch).await?;
    }

    Ok(())
}

/// Convert HTTP RPC URL to WebSocket URL
/// Test if an RPC endpoint is available by trying to get the current slot
async fn test_rpc_available(rpc: &RpcClient) -> bool {
    match rpc.get_slot().await {
        Ok(_) => true,
        Err(_) => false,
    }
}

/// Try to find an available RPC from a list of URLs
async fn find_available_rpc(urls: &[String]) -> Result<(usize, Arc<RpcClient>), anyhow::Error> {
    for (index, url) in urls.iter().enumerate() {
        let rpc = Arc::new(RpcClient::new(url.clone()));
        if test_rpc_available(&rpc).await {
            tracing::info!("Using RPC endpoint: {} (index: {})", url, index);
            return Ok((index, rpc));
        } else {
            tracing::warn!("RPC endpoint unavailable: {} (index: {})", url, index);
        }
    }
    Err(anyhow::anyhow!("No available RPC endpoints found"))
}

fn rpc_url_to_ws_url(rpc_url: &str) -> String {
    rpc_url
        .replace("https://", "wss://")
        .replace("http://", "ws://")
}

/// Parse account data from WebSocket notification
fn parse_account_data(account_data: &str) -> Result<Vec<u8>, anyhow::Error> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    STANDARD
        .decode(account_data)
        .map_err(|e| anyhow::anyhow!("Failed to decode base64: {}", e))
}

/// Display the deployed data in a formatted grid
async fn display_deployed_grid(
    treasury: &Treasury,
    board: &Board,
    round: &Round,
    clock: &Clock,
    app_state: &AppState,
) {
    app_state
        .data
        .write()
        .await
        .update(round.clone(), board.clone(), clock.clone(), treasury.clone());
    // Clear screen (works on most terminals)
    print!("\x1B[2J\x1B[1;1H");

    // Print header
    println!("╔════════════════════════════════════════════════════════════╗");
    println!(
        "║  Round {} - Deployed SOL (5x5 Grid) [WebSocket]         ║",
        board.round_id
    );
    println!("╚════════════════════════════════════════════════════════════╝");
    println!();

    // Print 5x5 grid
    println!("┌─────────────────────────┬─────────────────────────┬─────────────────────────┬─────────────────────────┬─────────────────────────┐");
    for row in 0..5 {
        // Print square numbers
        print!("│");
        for col in 0..5 {
            let idx = row * 5 + col;
            print!("        Square {:2}        │", idx);
        }
        println!();
        // Print deployed amounts
        print!("│");
        for col in 0..5 {
            let idx = row * 5 + col;
            let deployed_sol = lamports_to_sol(round.deployed[idx]);
            print!(" {:>12.6}({}) SOL   │", deployed_sol, round.count[idx]);
        }
        println!();
        if row < 4 {
            println!(
                "├─────────────────────────┼─────────────────────────┼─────────────────────────┼─────────────────────────┼─────────────────────────┤"
            );
        }
    }
    println!("└─────────────────────────┴─────────────────────────┴─────────────────────────┴─────────────────────────┴─────────────────────────┘");
    println!();

    // Print summary
    let total_deployed = round.deployed.iter().sum::<u64>();
    println!("Total Deployed: {:.6} SOL", lamports_to_sol(total_deployed));
    println!(
        "Total Deployed (from round): {:.6} SOL",
        lamports_to_sol(round.total_deployed)
    );
    println!("Current Slot: {}, start slot: {}, end slot: {}", clock.slot, board.start_slot, board.end_slot);
    println!(
        "Round Expires At: {} ({} slots remaining)",
        round.expires_at,
        round.expires_at.saturating_sub(clock.slot)
    );
    println!();
    println!("Press Ctrl+C to exit");
}

#[derive(Clone)]
struct WatchDeployed {
    _port: u16,
    board: Board,
    round: Round,
    clock: Clock,
    treasury: Treasury,

    write: Arc<Mutex<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>,
    read: Option<Arc<Mutex<SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>>>>,
    round_subscription_id: Option<u64>,
    board_subscription_id: Option<u64>,

    rpc: Arc<RpcClient>,
    rpc_urls: Vec<String>,
    current_rpc_index: Arc<Mutex<usize>>,
    redis_client: Arc<RedisClient>,
    http_state: AppState,

    last_snapshot_round_id: Option<u64>,
    last_round_update_time: Arc<Mutex<std::time::Instant>>,
}

impl WatchDeployed {
    async fn new(port: u16, urls: Vec<String>) -> Result<Self, anyhow::Error> {
        // Find an available RPC endpoint
        let (mut initial_index, mut rpc) = find_available_rpc(&urls).await?;
        
        let board = get_board(&rpc).await?;
        let round = get_round(&rpc, board.round_id).await?;
        let clock = get_clock(&rpc).await?;
        let treasury = get_treasury(&rpc).await?;
        let initial_data = RoundBoardData::new(round, board, clock.clone(), treasury);
        let redis_client = Arc::new(
            RedisClient::new(
                &std::env::var("REDIS_URL")
                    .map_err(|_| anyhow::anyhow!("Missing REDIS_URL env var"))?
            )
            .map_err(|e| anyhow::anyhow!("Failed to create Redis client: {}", e))?
        );

        tracing::info!("Start round info, round_id: {}, board_id: {}, start_slot: {}, current_slot: {}, end_slot: {}", round.id, board.round_id, board.start_slot, clock.slot, board.end_slot);

        let app_state = AppState {
            data: Arc::new(RwLock::new(initial_data)),
            redis_client: Some(redis_client.clone()),
        };
        // Clone state for HTTP server task
        let http_state = app_state.clone();

        // Initialize winning tiles cache
        winning_tile::init_winning_tiles_cache(None, 300, 60, rpc.clone(), redis_client.clone())
            .await?;

        tracing::info!("Winning tiles cache initialized");

        // Start HTTP server in background task
        let http_state_clone = http_state.clone();
        tokio::spawn(async move {
            if let Err(e) = http_server::start_http_server(http_state_clone, port).await {
                tracing::error!("HTTP server error: {}", e);
            }
            tracing::info!("HTTP server started on port {}", port);
        });

        // Try to connect WebSocket, if failed try next RPC
        let mut ws_connected = false;
        let mut ws_stream_result = None;
        
        for attempt in 0..urls.len() {
            let try_index = (initial_index + attempt) % urls.len();
            let wss_url = rpc_url_to_ws_url(&urls[try_index]);
            
            match connect_async(&wss_url).await {
                Ok((ws_stream, _)) => {
                    tracing::info!("WebSocket connected to: {} (index: {})", urls[try_index], try_index);
                    ws_stream_result = Some(ws_stream);
                    // If we switched to a different RPC for WebSocket, update rpc and index
                    if try_index != initial_index {
                        tracing::info!("Switched to RPC endpoint: {} (index: {}) for WebSocket", urls[try_index], try_index);
                        rpc = Arc::new(RpcClient::new(urls[try_index].clone()));
                        initial_index = try_index;
                    }
                    ws_connected = true;
                    break;
                }
                Err(e) => {
                    tracing::warn!("Failed to connect WebSocket to {} (index: {}): {}", urls[try_index], try_index, e);
                    if attempt < urls.len() - 1 {
                        tracing::info!("Trying next RPC endpoint...");
                    }
                }
            }
        }
        
        if !ws_connected {
            return Err(anyhow::anyhow!("Failed to connect WebSocket to any RPC endpoint"));
        }
        
        let (write, read) = match ws_stream_result {
            Some(stream) => stream.split(),
            None => {
                return Err(anyhow::anyhow!("WebSocket stream not available"));
            }
        };

        Ok(Self {
            _port: port,
            rpc,
            rpc_urls: urls,
            current_rpc_index: Arc::new(Mutex::new(initial_index)),
            board,
            round,
            clock,
            treasury,
            write: Arc::new(Mutex::new(write)),
            read: Some(Arc::new(Mutex::new(read))),
            http_state,
            redis_client,
            round_subscription_id: None,
            board_subscription_id: None,
            last_snapshot_round_id: Some(0),
            last_round_update_time: Arc::new(Mutex::new(std::time::Instant::now())),
        })
    }

    /// Switch to the next round when timeout detected
    async fn switch_to_next_round(&mut self, new_round_id: u64) -> Result<(), anyhow::Error> {
        tracing::info!("Switching from round {} to round {}", self.round.id, new_round_id);
        
        // Save old round info
        let old_round_id = self.round.id;
        let old_round_subscription_id = self.round_subscription_id;
        
        // Get new board and round data
        let new_board = get_board(&self.rpc).await?;
        let new_round = get_round(&self.rpc, new_round_id).await?;
        let new_clock = get_clock(&self.rpc).await?;
        
        // Snapshot old round if needed
        if old_round_id > self.last_snapshot_round_id.unwrap_or(0)
            && self.round.slot_hash != [0; 32] && self.round.slot_hash != [u8::MAX; 32]
        {
            let rpc_clone = self.rpc.clone();
            let redis_clone = self.redis_client.clone();
            let round = self.round.clone();
            let board = self.board.clone();
            tokio::spawn(async move {
                if let Err(e) = http_server::snapshot_round_to_redis(
                    rpc_clone,
                    redis_clone,
                    round,
                    board,
                )
                .await
                {
                    tracing::error!("Error snapshotting round {}: {}", round.id, e);
                }
            });
        }
        
        // Unsubscribe from old round
        if let Some(old_sub_id) = old_round_subscription_id {
            if let Err(e) = self.unsubscribe_account(
                ore_api::state::round_pda(old_round_id).0.to_string(),
                old_sub_id,
            ).await {
                tracing::warn!("Failed to unsubscribe from old round {}: {}", old_round_id, e);
            } else {
                tracing::info!("Unsubscribed from old round {}", old_round_id);
            }
        }
        
        // Subscribe to new round
        if let Err(e) = self.subscribe_account(
            ore_api::state::round_pda(new_round_id).0.to_string()
        ).await {
            tracing::error!("Failed to subscribe to new round {}: {}", new_round_id, e);
            return Err(anyhow::anyhow!("Failed to subscribe to new round: {}", e));
        } else {
            tracing::info!("Subscribed to new round {}", new_round_id);
        }
        
        // Update state
        self.board = new_board;
        self.round = new_round;
        self.clock = new_clock;
        self.last_snapshot_round_id = Some(new_round_id);
        // Update last round update time
        {
            let mut last_update = self.last_round_update_time.lock().await;
            *last_update = std::time::Instant::now();
        }
        
        // Update HTTP state
        self.http_state.data.write().await.update(
            self.round.clone(),
            self.board.clone(),
            self.clock.clone(),
            self.treasury.clone(),
        );
        
        // Display new round grid
        display_deployed_grid(
            &self.treasury,
            &self.board,
            &self.round,
            &self.clock,
            &self.http_state,
        )
        .await;
        
        tracing::info!("Successfully switched to round {}", new_round_id);
        Ok(())
    }

    /// Switch to the next available RPC endpoint
    async fn switch_to_next_rpc(&mut self) -> Result<(), anyhow::Error> {
        let start_index = {
            let current_index = self.current_rpc_index.lock().await;
            *current_index
        };
        let mut next_index = start_index;
        
        // Try all RPC endpoints starting from the next one
        loop {
            next_index = (next_index + 1) % self.rpc_urls.len();
            
            if next_index == start_index {
                // We've tried all endpoints, none are available
                return Err(anyhow::anyhow!("No available RPC endpoints found"));
            }
            
            let url = &self.rpc_urls[next_index];
            let new_rpc = Arc::new(RpcClient::new(url.clone()));
            
            if test_rpc_available(&new_rpc).await {
                tracing::info!("Switched to RPC endpoint: {} (index: {})", url, next_index);
                
                // Update the RPC client
                self.rpc = new_rpc;
                
                // Update the index
                {
                    let mut current_index = self.current_rpc_index.lock().await;
                    *current_index = next_index;
                }
                
                // Reconnect WebSocket
                let wss_url = rpc_url_to_ws_url(url);
                match connect_async(&wss_url).await {
                    Ok((ws_stream, _)) => {
                        let (write, read) = ws_stream.split();
                        *self.write.lock().await = write;
                        if let Some(read_mutex) = &self.read {
                            *read_mutex.lock().await = read;
                        } else {
                            self.read = Some(Arc::new(Mutex::new(read)));
                        }
                        
                        // Resubscribe to accounts (errors are logged but don't fail the switch)
                        if let Err(e) = self.subscribe_account(ore_api::state::board_pda().0.to_string()).await {
                            tracing::warn!("Failed to subscribe to board account after RPC switch: {}", e);
                        }
                        if let Err(e) = self.subscribe_account(ore_api::state::round_pda(self.round.id).0.to_string()).await {
                            tracing::warn!("Failed to subscribe to round account after RPC switch: {}", e);
                        }
                        
                        return Ok(());
                    }
                    Err(e) => {
                        tracing::warn!("Failed to connect WebSocket for {}: {}", url, e);
                        continue;
                    }
                }
            } else {
                tracing::warn!("RPC endpoint unavailable: {} (index: {})", url, next_index);
            }
        }
    }

    async fn subscribe_account(&mut self, account: String) -> Result<(), anyhow::Error> {
        tracing::info!("Subscribing to account: {}", account);

        let subscribe_account = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "accountSubscribe",
            "params": [account, { "encoding": "base64", "commitment": "confirmed" }]
        });

        self.write
            .lock()
            .await
            .send(Message::Text(subscribe_account.to_string()))
            .await?;

        Ok(())
    }

    async fn unsubscribe_account(
        &mut self,
        account: String,
        subscribe_id: u64,
    ) -> Result<(), anyhow::Error> {
        tracing::info!("Unsubscribing from account: {}", account);
        let unsubscribe_account = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "accountUnsubscribe",
            "params": [subscribe_id]
        });

        self.write
            .lock()
            .await
            .send(Message::Text(unsubscribe_account.to_string()))
            .await?;
        Ok(())
    }

    async fn watch_deployed_scheduler(&mut self) -> Result<(), anyhow::Error> {
        // Subscribe to accounts initially
        if let Err(e) = self.subscribe_account(ore_api::state::board_pda().0.to_string()).await {
            tracing::error!("Failed to subscribe to board account initially: {}", e);
            return Err(anyhow::anyhow!("Failed to subscribe to board account: {}", e));
        }
        if let Err(e) = self.subscribe_account(ore_api::state::round_pda(self.round.id).0.to_string()).await {
            tracing::error!("Failed to subscribe to round account initially: {}", e);
            return Err(anyhow::anyhow!("Failed to subscribe to round account: {}", e));
        }

        // Initialize last round update time
        {
            let mut last_update = self.last_round_update_time.lock().await;
            *last_update = std::time::Instant::now();
        }

        // Create channel for monitor to signal new round detection
        let (tx, mut rx) = mpsc::channel::<u64>(1);

        // Start background task to monitor round update timeout
        let wd_clone = self.clone();
        let monitor_tx = tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                
                let last_update_time = {
                    let last_update = wd_clone.last_round_update_time.lock().await;
                    *last_update
                };
                
                // Check if round hasn't been updated for more than 2 minutes
                if last_update_time.elapsed() > Duration::from_secs(120) {
                    tracing::warn!(
                        "Round {} hasn't been updated for {} seconds, checking for new round...",
                        wd_clone.round.id,
                        last_update_time.elapsed().as_secs()
                    );
                    
                    // Try to get latest board to check if there's a new round
                    match get_board(&wd_clone.rpc).await {
                        Ok(new_board) => {
                            if new_board.round_id != wd_clone.board.round_id {
                                tracing::info!(
                                    "Found new round: {} -> {}, switching...",
                                    wd_clone.board.round_id,
                                    new_board.round_id
                                );
                                // Signal to switch round
                                let _ = monitor_tx.send(new_board.round_id).await;
                            } else {
                                // Board round_id is same, but round might have changed
                                // Try to get the round to see if it's different
                                match get_round(&wd_clone.rpc, new_board.round_id).await {
                                    Ok(new_round) => {
                                        if new_round.id != wd_clone.round.id {
                                            tracing::info!(
                                                "Found new round ID: {} -> {}, switching...",
                                                wd_clone.round.id,
                                                new_round.id
                                            );
                                            let _ = monitor_tx.send(new_round.id).await;
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!("Failed to get round {}: {}", new_board.round_id, e);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to get board for timeout check: {}", e);
                        }
                    }
                }
            }
        });

        // Main message handling loop with automatic failover
        loop {
            // Extract read from self to use in spawned task
            let read = match self.read.take() {
                Some(r) => r,
                None => {
                    tracing::error!("Read stream not available, attempting to reconnect...");
                    if let Err(e) = self.switch_to_next_rpc().await {
                        tracing::error!("Failed to reconnect: {}", e);
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                    match self.read.take() {
                        Some(r) => r,
                        None => {
                            tracing::error!("Read stream still not available after reconnect, retrying...");
                            tokio::time::sleep(Duration::from_secs(2)).await;
                            continue;
                        }
                    }
                }
            };

            // start new task to process messages
            let read_clone = read.clone();
            let wd_clone = self.clone();
            let handle = tokio::spawn(async move {
                if let Err(e) = Self::handle_message(read_clone, wd_clone).await {
                    tracing::error!("Error processing messages: {}", e);
                }
            });

            // Wait for either message handler or monitor signal
            tokio::select! {
                result = handle => {
                    if let Err(e) = result {
                        tracing::error!("Task join error: {}", e);
                    }
                }
                new_round_id = rx.recv() => {
                    if let Some(round_id) = new_round_id {
                        tracing::info!("Monitor detected new round: {}, switching...", round_id);
                        if let Err(e) = self.switch_to_next_round(round_id).await {
                            tracing::error!("Failed to switch to next round: {}", e);
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            continue;
                        }
                        // Reset last update time after switching
                        {
                            let mut last_update = self.last_round_update_time.lock().await;
                            *last_update = std::time::Instant::now();
                        }
                        continue;
                    }
                }
            }
            
            // After error, try to reconnect
            tracing::info!("Attempting to reconnect WebSocket...");
            tokio::time::sleep(Duration::from_secs(2)).await;
            
            // Try to switch to next RPC
            if let Err(e) = self.switch_to_next_rpc().await {
                tracing::error!("Failed to switch RPC: {}, will retry...", e);
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
            
            // Resubscribe after reconnection
            if let Err(e) = self.subscribe_account(ore_api::state::board_pda().0.to_string()).await {
                tracing::error!("Failed to resubscribe to board account: {}", e);
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            if let Err(e) = self.subscribe_account(ore_api::state::round_pda(self.round.id).0.to_string()).await {
                tracing::error!("Failed to resubscribe to round account: {}", e);
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        }
    }

    async fn handle_message(
        read: Arc<Mutex<SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>>>,
        mut wd: WatchDeployed,
    ) -> Result<(), anyhow::Error> {
        while let Some(msg) = read.lock().await.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    // tracing::info!("Handling message: {}", text);
                    match wd.handle_subscribe_message(text.clone()).await {
                        Ok(_) => {}
                        Err(e) => {
                            tracing::error!("Error handling subscribe message: {}", e);
                        }
                    }
                    match wd.handle_account_notification(text).await {
                        Ok(_) => {}
                        Err(e) => {
                            tracing::error!("Error handling account notification: {}", e);
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    tracing::warn!("WebSocket connection closed");
                    // Return error to trigger reconnection in watch_deployed_scheduler
                    return Err(anyhow::anyhow!("WebSocket connection closed"));
                }
                Err(e) => {
                    tracing::warn!("WebSocket error: {}", e);
                    // Return error to trigger reconnection in watch_deployed_scheduler
                    return Err(anyhow::anyhow!("WebSocket error: {}", e));
                }
                _ => {}
            }
        }
        Ok(())
    }

    async fn handle_subscribe_message(&mut self, text: String) -> Result<(), anyhow::Error> {
        let value: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to parse JSON: {}", e));
            }
        };

        // Check if this is a subscription confirmation (response to our subscribe request)
        if let Some(id) = value.get("id") {
            if let Some(result) = value.get("result") {
                // result maybe a u64 or a bool, if is u64, it is a subscription confirmation
                // else if is bool, it is a unsubscribe confirmation
                if result.is_u64() {
                    let sub_id = match result.as_u64() {
                        Some(id) => id,
                        None => {
                            tracing::warn!("Failed to parse subscription ID as u64");
                            return Ok(());
                        }
                    };
                    let request_id = id.as_u64().unwrap_or(0);
                    if request_id == 1 {
                        if self.board_subscription_id.is_none() {
                            self.board_subscription_id = Some(sub_id);
                            // self.round_subscription_id = Some(sub_id);
                        } else {
                            self.round_subscription_id = Some(sub_id);
                        }
                    }
                } else if result.is_boolean() {
                    let is_success = result.as_bool().unwrap_or(false);
                    if is_success {
                        tracing::info!("UnSubscription confirmed");
                    } else {
                        tracing::error!("UnSubscription failed");
                    }
                }
            }
        }

        Ok(())
    }

    async fn handle_account_notification(&mut self, text: String) -> Result<(), anyhow::Error> {
        // Check if this is an account notification (method field indicates notification)
        let value: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to parse JSON: {}", e));
            }
        };

        // get method, params, subscription, account_data
        let method = value
            .get("method")
            .ok_or_else(|| anyhow::anyhow!("Method not found"))?;
        let params = value
            .get("params")
            .ok_or_else(|| anyhow::anyhow!("Params not found"))?;
        let subscription = params
            .get("subscription")
            .ok_or_else(|| anyhow::anyhow!("Subscription not found"))?;

        let data_array = params
            .get("result")
            .ok_or_else(|| anyhow::anyhow!("Account not found"))?
            .get("value")
            .ok_or_else(|| anyhow::anyhow!("Account data value not found"))?
            .get("data")
            .ok_or_else(|| anyhow::anyhow!("Data not found"))?
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Data is not an array"))?;

        if method.as_str() == Some("accountNotification") {
            if data_array.len() >= 1 {
                let account_data_str = data_array[0]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Account data is not a string"))?;
                // Decode account data
                match parse_account_data(account_data_str) {
                    Ok(account_bytes) => {
                        // Determine which account this is by checking subscription ID
                        let subscription_id = subscription.as_u64().unwrap_or(0);

                        // Check if this is round account (compare with stored subscription IDs)
                        let is_round_by_sub = self
                            .round_subscription_id
                            .map(|id| id == subscription_id)
                            .unwrap_or(false);
                        let is_board_by_sub = self
                            .board_subscription_id
                            .map(|id| id == subscription_id)
                            .unwrap_or(false);

                        let account_size = account_bytes.len();
                        let might_be_round = account_size > 100; // Round is much larger
                        let might_be_board = account_size < 50; // Board is smaller

                        // Try round first if subscription matches or size suggests it
                        if is_round_by_sub || (might_be_round && !is_board_by_sub) {
                            match Round::try_from_bytes(&account_bytes) {
                                Ok(parsed_round) => {
                                    tracing::info!("parsed_round: {:?}", parsed_round);
                                    if parsed_round.id == self.round.id {
                                        // Try to get clock with failover
                                        match get_clock(&self.rpc).await {
                                            Ok(clock) => {
                                                self.clock = clock;
                                            }
                                            Err(e) => {
                                                tracing::warn!("Failed to get clock: {}, attempting RPC failover...", e);
                                                if let Err(switch_err) = self.switch_to_next_rpc().await {
                                                    return Err(anyhow::anyhow!("Failed to switch RPC: {}", switch_err));
                                                }
                                                self.clock = get_clock(&self.rpc).await?;
                                            }
                                        }
                                        if self.round.deployed != parsed_round.deployed
                                            || self.round.count != parsed_round.count 
                                            || self.round.slot_hash != parsed_round.slot_hash
                                        {
                                            display_deployed_grid(
                                                &self.treasury,
                                                &self.board,
                                                &parsed_round,
                                                &self.clock,
                                                &self.http_state,
                                            )
                                            .await;
                                            self.round = *parsed_round;
                                            // Update last round update time
                                            {
                                                let mut last_update = self.last_round_update_time.lock().await;
                                                *last_update = std::time::Instant::now();
                                            }
                                        }

                                        if parsed_round.slot_hash != [0; 32] && parsed_round.slot_hash != [u8::MAX; 32] {
                                            // snapshot the previous round
                                            // Try to get treasury with failover
                                            match get_treasury(&self.rpc).await {
                                                Ok(treasury) => {
                                                    self.treasury = treasury;
                                                }
                                                Err(e) => {
                                                    tracing::warn!("Failed to get treasury: {}, attempting RPC failover...", e);
                                                    if let Err(switch_err) = self.switch_to_next_rpc().await {
                                                        return Err(anyhow::anyhow!("Failed to switch RPC: {}", switch_err));
                                                    }
                                                    self.treasury = get_treasury(&self.rpc).await?;
                                                }
                                            }
                                            let rpc_clone = self.rpc.clone();
                                            let redis_clone = self.redis_client.clone();
                                            let round = self.round.clone();
                                            let board = self.board.clone();
                                            tokio::spawn(async move {
                                                if let Err(e) = http_server::snapshot_round_to_redis(rpc_clone, redis_clone, round, board).await {
                                                    tracing::error!("Error snapshotting round {}: {}", round.id, e);
                                                }
                                            });
                                        }
                                    } else {
                                        // maybe is a new round data - round ID changed
                                        tracing::warn!(
                                            "Round ID changed: old={}, new={}",
                                            self.round.id,
                                            parsed_round.id
                                        );
                                        
                                        // Save old round ID for unsubscription
                                        let old_round_id = self.round.id;
                                        let old_round_subscription_id = self.round_subscription_id;
                                        
                                        // Update round data
                                        self.round = *parsed_round;
                                        // Update last round update time
                                        {
                                            let mut last_update = self.last_round_update_time.lock().await;
                                            *last_update = std::time::Instant::now();
                                        }
                                        
                                        // Update board.round_id if needed
                                        if self.board.round_id != parsed_round.id {
                                            tracing::info!("Updating board.round_id from {} to {}", self.board.round_id, parsed_round.id);
                                            // Note: We can't directly modify board.round_id as it's part of the struct
                                            // We need to get the board again or wait for board update notification
                                            // For now, we'll subscribe to the new round and unsubscribe from the old one
                                        }
                                        
                                        // Unsubscribe from old round if we have subscription ID
                                        if let Some(old_sub_id) = old_round_subscription_id {
                                            if let Err(e) = self.unsubscribe_account(
                                                ore_api::state::round_pda(old_round_id).0.to_string(),
                                                old_sub_id,
                                            ).await {
                                                tracing::warn!("Failed to unsubscribe from old round account {}: {}", old_round_id, e);
                                            } else {
                                                tracing::info!("Unsubscribed from old round {}", old_round_id);
                                            }
                                        }
                                        
                                        // Subscribe to new round
                                        if let Err(e) = self.subscribe_account(
                                            ore_api::state::round_pda(parsed_round.id).0.to_string()
                                        ).await {
                                            tracing::error!("Failed to subscribe to new round account {}: {}", parsed_round.id, e);
                                        } else {
                                            tracing::info!("Subscribed to new round {}", parsed_round.id);
                                        }
                                        
                                        // Try to get clock with failover
                                        match get_clock(&self.rpc).await {
                                            Ok(clock) => {
                                                self.clock = clock;
                                            }
                                            Err(e) => {
                                                tracing::warn!("Failed to get clock: {}, attempting RPC failover...", e);
                                                if let Err(switch_err) = self.switch_to_next_rpc().await {
                                                    return Err(anyhow::anyhow!("Failed to switch RPC: {}", switch_err));
                                                }
                                                self.clock = get_clock(&self.rpc).await?;
                                            }
                                        }
                                        
                                        display_deployed_grid(
                                            &self.treasury,
                                            &self.board,
                                            &parsed_round,
                                            &self.clock,
                                            &self.http_state,
                                        )
                                        .await;
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("Failed to parse as Round: {:?}", e);
                                    tracing::error!(
                                        "First 32 bytes: {:?}",
                                        &account_bytes[..account_bytes.len().min(32)]
                                    );
                                    // Try board as fallback
                                    if might_be_board {
                                        tracing::info!("Trying to parse as Board instead...");
                                        let parsed_board = Board::try_from_bytes(&account_bytes)?;
                                        // board round id update, we should snapshot the previous round and subscribe to the new round
                                        if parsed_board.round_id != self.board.round_id {
                                            self.last_snapshot_round_id = Some(self.round.id);
                                            // Try to get treasury with failover
                                            match get_treasury(&self.rpc).await {
                                                Ok(treasury) => {
                                                    self.treasury = treasury;
                                                }
                                                Err(e) => {
                                                    tracing::warn!("Failed to get treasury: {}, attempting RPC failover...", e);
                                                    if let Err(switch_err) = self.switch_to_next_rpc().await {
                                                        return Err(anyhow::anyhow!("Failed to switch RPC: {}", switch_err));
                                                    }
                                                    self.treasury = get_treasury(&self.rpc).await?;
                                                }
                                            }
                                            tracing::warn!(
                                                "1、Round changed from {} to {}",
                                                self.board.round_id,
                                                parsed_board.round_id
                                            );
                                            if self.round.id > self.last_snapshot_round_id.unwrap_or(0)
                                            && self.round.slot_hash != [0; 32] && self.round.slot_hash != [u8::MAX; 32]
                                            {
                                                // Snapshot the previous round before moving to new one
                                                let rpc_clone = self.rpc.clone();
                                                let redis_clone = self.redis_client.clone();
                                                let board = self.board.clone();
                                                let round = self.round.clone();
                                                tokio::spawn(async move {
                                                    if let Err(e) =
                                                        http_server::snapshot_round_to_redis(
                                                            rpc_clone,
                                                            redis_clone,
                                                            round,
                                                            board,
                                                        )
                                                        .await
                                                    {
                                                        tracing::error!(
                                                            "Error snapshotting round {}: {}",
                                                            round.id,
                                                            e
                                                        );
                                                    }
                                                });
                                            }

                                            // subscribe to the new round and unsubscribe from the previous round
                                            let round_id = self.board.round_id;
                                            let round_subscription_id = self.round_subscription_id;
                                            
                                            // Get the new round data
                                            match get_round(&self.rpc, parsed_board.round_id).await {
                                                Ok(new_round) => {
                                                    tracing::info!("Got new round data for round {}", parsed_board.round_id);
                                                    self.round = new_round;
                                                    // Update last round update time
                                                    {
                                                        let mut last_update = self.last_round_update_time.lock().await;
                                                        *last_update = std::time::Instant::now();
                                                    }
                                                }
                                                Err(e) => {
                                                    tracing::warn!("Failed to get new round data for round {}: {}, will try again later", parsed_board.round_id, e);
                                                    // Continue anyway, subscription will trigger updates
                                                }
                                            }
                                            
                                            // Subscribe to new round first
                                            if let Err(e) = self.subscribe_account(
                                                ore_api::state::round_pda(parsed_board.round_id)
                                                    .0
                                                    .to_string(),
                                            ).await {
                                                tracing::error!("Failed to subscribe to new round {}: {}", parsed_board.round_id, e);
                                            } else {
                                                tracing::info!("Subscribed to new round {}", parsed_board.round_id);
                                            }
                                            
                                            // Update board
                                            self.board = *parsed_board;
                                            self.board.end_slot = self.board.start_slot + 150;
                                            
                                            // Unsubscribe from old round if we have subscription ID
                                            if let Some(old_sub_id) = round_subscription_id {
                                                if let Err(e) = self.unsubscribe_account(
                                                    ore_api::state::round_pda(round_id).0.to_string(),
                                                    old_sub_id,
                                                ).await {
                                                    tracing::warn!("Failed to unsubscribe from old round {}: {}", round_id, e);
                                                } else {
                                                    tracing::info!("Unsubscribed from old round {}", round_id);
                                                }
                                            }
                                        } else {
                                            // Try to get treasury with failover
                                            match get_treasury(&self.rpc).await {
                                                Ok(treasury) => {
                                                    self.treasury = treasury;
                                                }
                                                Err(e) => {
                                                    tracing::warn!("Failed to get treasury: {}, attempting RPC failover...", e);
                                                    if let Err(switch_err) = self.switch_to_next_rpc().await {
                                                        return Err(anyhow::anyhow!("Failed to switch RPC: {}", switch_err));
                                                    }
                                                    self.treasury = get_treasury(&self.rpc).await?;
                                                }
                                            }
                                            self.board = *parsed_board;
                                            self.board.end_slot = self.board.start_slot + 150;
                                            self.http_state.data.write().await.update(self.round.clone(), *parsed_board, self.clock.clone(), self.treasury.clone());
                                        }
                                    }
                                }
                            }
                        } else if is_board_by_sub || might_be_board {
                            // Board account update
                            let parsed_board = Board::try_from_bytes(&account_bytes)?;
                            tracing::info!("parsed_board: {:?}", parsed_board);
                            if parsed_board.round_id != self.board.round_id {
                                tracing::warn!(
                                    "2、Board check, Round changed from {} to {}",
                                    self.board.round_id,
                                    parsed_board.round_id
                                );
                                // Try to get treasury with failover
                                match get_treasury(&self.rpc).await {
                                    Ok(treasury) => {
                                        self.treasury = treasury;
                                    }
                                    Err(e) => {
                                        tracing::warn!("Failed to get treasury: {}, attempting RPC failover...", e);
                                        if let Err(switch_err) = self.switch_to_next_rpc().await {
                                            return Err(anyhow::anyhow!("Failed to switch RPC: {}", switch_err));
                                        }
                                        self.treasury = get_treasury(&self.rpc).await?;
                                    }
                                }

                                if parsed_board.round_id > self.last_snapshot_round_id.unwrap_or(0)
                                && self.round.slot_hash != [0; 32] && self.round.slot_hash != [u8::MAX; 32]
                                {
                                    // Snapshot the previous round before moving to new one
                                    self.last_snapshot_round_id = Some(parsed_board.round_id);
                                    let rpc_clone = self.rpc.clone();
                                    let redis_clone = self.redis_client.clone();
                                    let round = self.round.clone();
                                    let board = self.board.clone();
                                    tokio::spawn(async move {
                                        if let Err(e) = http_server::snapshot_round_to_redis(
                                            rpc_clone,
                                            redis_clone,
                                            round,
                                            board,
                                        )
                                        .await
                                        {
                                            tracing::error!(
                                                "Error snapshotting round {}: {}",
                                                round.id,
                                                e
                                            );
                                        }
                                    });
                                }

                                // subscribe to the new round and unsubscribe from the previous round
                                let board_round_id = self.board.round_id;
                                let round_subscription_id = self.round_subscription_id;

                                // Get the new round data
                                match get_round(&self.rpc, parsed_board.round_id).await {
                                    Ok(new_round) => {
                                        tracing::info!("Got new round data for round {}", parsed_board.round_id);
                                        self.round = new_round;
                                        // Update last round update time
                                        {
                                            let mut last_update = self.last_round_update_time.lock().await;
                                            *last_update = std::time::Instant::now();
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!("Failed to get new round data for round {}: {}, will try again later", parsed_board.round_id, e);
                                        // Continue anyway, subscription will trigger updates
                                    }
                                }

                                // Subscribe to new round first
                                if let Err(e) = self.subscribe_account(
                                    ore_api::state::round_pda(parsed_board.round_id)
                                        .0
                                        .to_string(),
                                ).await {
                                    tracing::error!("Failed to subscribe to new round {}: {}", parsed_board.round_id, e);
                                } else {
                                    tracing::info!("Subscribed to new round {}", parsed_board.round_id);
                                }
                                
                                // Update board
                                self.board = *parsed_board;
                                self.board.end_slot = self.board.start_slot + 150;

                                // Unsubscribe from old round if we have subscription ID
                                if let Some(old_sub_id) = round_subscription_id {
                                    if let Err(e) = self.unsubscribe_account(
                                        ore_api::state::round_pda(board_round_id).0.to_string(),
                                        old_sub_id,
                                    ).await {
                                        tracing::warn!("Failed to unsubscribe from old round {}: {}", board_round_id, e);
                                    } else {
                                        tracing::info!("Unsubscribed from old round {}", board_round_id);
                                    }
                                }
                            } else {
                                // Try to get treasury with failover
                                match get_treasury(&self.rpc).await {
                                    Ok(treasury) => {
                                        self.treasury = treasury;
                                    }
                                    Err(e) => {
                                        tracing::warn!("Failed to get treasury: {}, attempting RPC failover...", e);
                                        if let Err(switch_err) = self.switch_to_next_rpc().await {
                                            return Err(anyhow::anyhow!("Failed to switch RPC: {}", switch_err));
                                        }
                                        self.treasury = get_treasury(&self.rpc).await?;
                                    }
                                }
                                self.board = *parsed_board;
                                self.board.end_slot = self.board.start_slot + 150;
                                self.http_state.data.write().await.update(self.round.clone(), *parsed_board, self.clock.clone(), self.treasury.clone());
                            }
                        } else {
                            tracing::error!("Unknown subscription ID: {}", subscription_id);
                        }
                    }
                    Err(e) => {
                        // Ignore parse errors for now
                        tracing::error!("Failed to parse account data: {}", e);
                    }
                }
            }
        }

        Ok(())
    }
}


async fn log_automation(rpc: &RpcClient) -> Result<(), anyhow::Error> {
    let authority = std::env::var("AUTHORITY").expect("Missing AUTHORITY env var");
    let authority = Pubkey::from_str(&authority).expect("Invalid AUTHORITY");
    let address = automation_pda(authority).0;
    let automation = get_automation(rpc, address).await?;
    let account_balance = rpc.get_balance(&address).await?;
    let size = 8 + std::mem::size_of::<Automation>();
    let required_rent = Rent::default().minimum_balance(size);
    println!("Automation");
    println!("  address: {}", address);
    println!("  amount: {} SOL", lamports_to_sol(automation.amount));
    println!("  required rent: {} SOL", lamports_to_sol(required_rent));
    println!("  authority: {}", automation.authority);
    println!("  balance: {} SOL", lamports_to_sol(automation.balance));
    println!("  lamports: {} SOL", lamports_to_sol(account_balance));
    println!("  executor: {}", automation.executor);
    println!("  fee: {} SOL", lamports_to_sol(automation.fee));
    println!("  mask: {}", automation.mask);
    println!("  strategy: {}", automation.strategy);
    println!("  reload: {}", automation.reload);
    Ok(())
}

async fn log_automations(rpc: &RpcClient) -> Result<(), anyhow::Error> {
    let automations = get_automations(rpc).await?;
    for (i, (address, automation)) in automations.iter().enumerate() {
        println!("[{}/{}] {}", i + 1, automations.len(), address);
        println!("  authority: {}", automation.authority);
        println!("  balance: {}", automation.balance);
        println!("  executor: {}", automation.executor);
        println!("  fee: {}", automation.fee);
        println!("  mask: {}", automation.mask);
        println!("  strategy: {}", automation.strategy);
        println!();
    }
    Ok(())
}

async fn log_treasury(rpc: &RpcClient) -> Result<(), anyhow::Error> {
    let treasury_address = ore_api::state::treasury_pda().0;
    let treasury = get_treasury(rpc).await?;
    println!("Treasury");
    println!("  address: {}", treasury_address);
    println!("  buffer_a: {}", treasury.buffer_a);
    println!("  balance: {} SOL", lamports_to_sol(treasury.balance));
    println!(
        "  motherlode: {} ORE",
        amount_to_ui_amount(treasury.motherlode, TOKEN_DECIMALS)
    );
    println!(
        "  miner_rewards_factor: {}",
        treasury.miner_rewards_factor.to_i80f48().to_string()
    );
    println!(
        "  stake_rewards_factor: {}",
        treasury.stake_rewards_factor.to_i80f48().to_string()
    );
    println!("  buffer_b: {}", treasury.buffer_b);
    println!(
        "  total_refined: {} ORE",
        amount_to_ui_amount(treasury.total_refined, TOKEN_DECIMALS)
    );
    println!(
        "  total_staked: {} ORE",
        amount_to_ui_amount(treasury.total_staked, TOKEN_DECIMALS)
    );
    println!(
        "  total_unclaimed: {} ORE",
        amount_to_ui_amount(treasury.total_unclaimed, TOKEN_DECIMALS)
    );
    Ok(())
}

async fn log_round(rpc: &RpcClient) -> Result<(), anyhow::Error> {
    let id = std::env::var("ID").expect("Missing ID env var");
    let id = u64::from_str(&id).expect("Invalid ID");
    let round_address = round_pda(id).0;
    let round = get_round(rpc, id).await?;
    let rng = round.rng();
    println!("Round");
    println!("  Address: {}", round_address);
    println!("  Count: {:?}", round.count);
    println!("  Deployed: {:?}", round.deployed);
    println!("  Expires at: {}", round.expires_at);
    println!("  Id: {:?}", round.id);
    println!(
        "  Motherlode: {} ORE",
        amount_to_ui_amount(round.motherlode, TOKEN_DECIMALS)
    );
    println!("  Rent payer: {}", round.rent_payer);
    println!("  Slot hash: {:?}", round.slot_hash);
    println!("  Top miner: {:?}", round.top_miner);
    println!(
        "  Top miner reward: {} ORE",
        amount_to_ui_amount(round.top_miner_reward, TOKEN_DECIMALS)
    );
    println!("  Total miners: {}", round.total_miners);
    println!(
        "  Total deployed: {} SOL",
        lamports_to_sol(round.total_deployed)
    );
    println!(
        "  Total vaulted: {} SOL",
        lamports_to_sol(round.total_vaulted)
    );
    println!(
        "  Total winnings: {} SOL",
        lamports_to_sol(round.total_winnings)
    );
    if let Some(rng) = rng {
        println!("  Winning square: {}", round.winning_square(rng));
    }
    // if round.slot_hash != [0; 32] {
    //     println!("  Winning square: {}", get_winning_square(&round.slot_hash));
    // }
    Ok(())
}

async fn log_sync_round(rpc: &RpcClient) -> Result<(), anyhow::Error> {
    // get start ID from redis, sync from provies ID to latest ID
    // default from 0 to the latest ID
    let id = std::env::var("ID").unwrap_or("0".to_string());
    let id = u64::from_str(&id).expect("Invalid ID");
    let round_address = round_pda(id).0;
    let round = get_round(rpc, id).await?;
    let rng = round.rng();
    println!("Round");
    println!("  Address: {}", round_address);
    println!("  Count: {:?}", round.count);
    println!("  Deployed: {:?}", round.deployed);
    println!("  Expires at: {}", round.expires_at);
    println!("  Id: {:?}", round.id);
    println!("  Motherlode: {}", round.motherlode);
    println!("  Rent payer: {}", round.rent_payer);
    println!("  Slot hash: {:?}", round.slot_hash);
    println!("  Top miner: {:?}", round.top_miner);
    println!("  Top miner reward: {}", round.top_miner_reward);
    println!("  Total deployed: {}", round.total_deployed);
    println!("  Total vaulted: {}", round.total_vaulted);
    println!("  Total winnings: {}", round.total_winnings);
    if let Some(rng) = rng {
        println!("  Winning square: {}", round.winning_square(rng));
    }
    // if round.slot_hash != [0; 32] {
    //     println!("  Winning square: {}", get_winning_square(&round.slot_hash));
    // }
    // store to redis
    Ok(())
}

async fn log_miner(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
) -> Result<(), anyhow::Error> {
    let authority = std::env::var("AUTHORITY").unwrap_or(payer.pubkey().to_string());
    let authority = Pubkey::from_str(&authority).expect("Invalid AUTHORITY");
    let miner_address = ore_api::state::miner_pda(authority).0;
    let miner = get_miner(&rpc, authority).await?;
    println!("Miner");
    println!("  address: {}", miner_address);
    println!("  authority: {}", authority);
    println!("  deployed: {:?}", miner.deployed);
    println!("  cumulative: {:?}", miner.cumulative);
    println!("  rewards_sol: {} SOL", lamports_to_sol(miner.rewards_sol));
    println!(
        "  rewards_ore: {} ORE",
        amount_to_ui_amount(miner.rewards_ore, TOKEN_DECIMALS)
    );
    println!(
        "  refined_ore: {} ORE",
        amount_to_ui_amount(miner.refined_ore, TOKEN_DECIMALS)
    );
    println!("  round_id: {}", miner.round_id);
    println!("  checkpoint_id: {}", miner.checkpoint_id);
    println!(
        "  lifetime_rewards_sol: {} SOL",
        lamports_to_sol(miner.lifetime_rewards_sol)
    );
    println!(
        "  lifetime_rewards_ore: {} ORE",
        amount_to_ui_amount(miner.lifetime_rewards_ore, TOKEN_DECIMALS)
    );
    Ok(())
}

async fn log_clock(rpc: &RpcClient) -> Result<(), anyhow::Error> {
    let clock = get_clock(&rpc).await?;
    println!("Clock");
    println!("  slot: {}", clock.slot);
    println!("  epoch_start_timestamp: {}", clock.epoch_start_timestamp);
    println!("  epoch: {}", clock.epoch);
    println!("  leader_schedule_epoch: {}", clock.leader_schedule_epoch);
    println!("  unix_timestamp: {}", clock.unix_timestamp);
    Ok(())
}

async fn log_config(rpc: &RpcClient) -> Result<(), anyhow::Error> {
    let config = get_config(&rpc).await?;
    println!("Config");
    println!("  admin: {}", config.admin);
    println!("  bury_authority: {}", config.bury_authority);
    println!("  fee_collector: {}", config.fee_collector);
    println!("  swap_program: {}", config.swap_program);
    println!("  var_address: {}", config.var_address);
    println!("  admin_fee: {}", config.admin_fee);
    Ok(())
}

async fn log_board(rpc: &RpcClient) -> Result<(), anyhow::Error> {
    let board = get_board(&rpc).await?;
    let clock = get_clock(&rpc).await?;
    print_board(board, &clock);
    Ok(())
}

fn print_board(board: Board, clock: &Clock) {
    let current_slot = clock.slot;
    println!("Board");
    println!("  Id: {:?}", board.round_id);
    println!("  Start slot: {}", board.start_slot);
    println!("  End slot: {}", board.end_slot);
    println!(
        "  Time remaining: {} sec",
        (board.end_slot.saturating_sub(current_slot) as f64) * 0.4
    );
    println!("  Epoch id: {:?}", board.epoch_id);
}

async fn get_automation(rpc: &RpcClient, address: Pubkey) -> Result<Automation, anyhow::Error> {
    let account = rpc.get_account(&address).await?;
    let automation = Automation::try_from_bytes(&account.data)?;
    Ok(*automation)
}

async fn get_automations(rpc: &RpcClient) -> Result<Vec<(Pubkey, Automation)>, anyhow::Error> {
    const REGOLITH_EXECUTOR: Pubkey = pubkey!("HNWhK5f8RMWBqcA7mXJPaxdTPGrha3rrqUrri7HSKb3T");
    let filter = RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
        56,
        &REGOLITH_EXECUTOR.to_bytes(),
    ));
    let automations = get_program_accounts::<Automation>(rpc, ore_api::ID, vec![filter]).await?;
    Ok(automations)
}

// async fn get_meteora_pool(rpc: &RpcClient, address: Pubkey) -> Result<Pool, anyhow::Error> {
//     let data = rpc.get_account_data(&address).await?;
//     let pool = Pool::from_bytes(&data)?;
//     Ok(pool)
// }

// async fn get_meteora_vault(rpc: &RpcClient, address: Pubkey) -> Result<Vault, anyhow::Error> {
//     let data = rpc.get_account_data(&address).await?;
//     let vault = Vault::from_bytes(&data)?;
//     Ok(vault)
// }

async fn get_board(rpc: &RpcClient) -> Result<Board, anyhow::Error> {
    let board_pda = ore_api::state::board_pda();
    let account = rpc.get_account(&board_pda.0).await?;
    let board = Board::try_from_bytes(&account.data)?;
    Ok(*board)
}

async fn get_var(rpc: &RpcClient, address: Pubkey) -> Result<Var, anyhow::Error> {
    let account = rpc.get_account(&address).await?;
    let var = Var::try_from_bytes(&account.data)?;
    Ok(*var)
}

async fn get_round(rpc: &RpcClient, id: u64) -> Result<Round, anyhow::Error> {
    let round_pda = ore_api::state::round_pda(id);
    let account = rpc.get_account(&round_pda.0).await?;
    let round = Round::try_from_bytes(&account.data)?;
    Ok(*round)
}

async fn get_treasury(rpc: &RpcClient) -> Result<Treasury, anyhow::Error> {
    let treasury_pda = ore_api::state::treasury_pda();
    let account = rpc.get_account(&treasury_pda.0).await?;
    let treasury = Treasury::try_from_bytes(&account.data)?;
    Ok(*treasury)
}

async fn get_config(rpc: &RpcClient) -> Result<Config, anyhow::Error> {
    let config_pda = ore_api::state::config_pda();
    let account = rpc.get_account(&config_pda.0).await?;
    let config = Config::try_from_bytes(&account.data)?;
    Ok(*config)
}

async fn get_miner(rpc: &RpcClient, authority: Pubkey) -> Result<Miner, anyhow::Error> {
    let miner_pda = ore_api::state::miner_pda(authority);
    let account = rpc.get_account(&miner_pda.0).await?;
    let miner = Miner::try_from_bytes(&account.data)?;
    Ok(*miner)
}

async fn get_clock(rpc: &RpcClient) -> Result<Clock, anyhow::Error> {
    let data = rpc.get_account_data(&solana_sdk::sysvar::clock::ID).await?;
    let clock = bincode::deserialize::<Clock>(&data)?;
    Ok(clock)
}

async fn get_stake(rpc: &RpcClient, authority: Pubkey) -> Result<Stake, anyhow::Error> {
    let stake_pda = ore_api::state::stake_pda(authority);
    let account = rpc.get_account(&stake_pda.0).await?;
    let stake = Stake::try_from_bytes(&account.data)?;
    Ok(*stake)
}

async fn get_rounds(rpc: &RpcClient) -> Result<Vec<(Pubkey, Round)>, anyhow::Error> {
    let rounds = get_program_accounts::<Round>(rpc, ore_api::ID, vec![]).await?;
    Ok(rounds)
}

#[allow(dead_code)]
async fn get_miners(rpc: &RpcClient) -> Result<Vec<(Pubkey, Miner)>, anyhow::Error> {
    let miners = get_program_accounts::<Miner>(rpc, ore_api::ID, vec![]).await?;
    Ok(miners)
}

async fn get_miners_participating(
    rpc: &RpcClient,
    round_id: u64,
) -> Result<Vec<(Pubkey, Miner)>, anyhow::Error> {
    let filter = RpcFilterType::Memcmp(Memcmp::new_base58_encoded(512, &round_id.to_le_bytes()));
    let miners = get_program_accounts::<Miner>(rpc, ore_api::ID, vec![filter]).await?;
    Ok(miners)
}

// fn get_winning_square(slot_hash: &[u8]) -> u64 {
//     // Use slot hash to generate a random u64
//     let r1 = u64::from_le_bytes(slot_hash[0..8].try_into().unwrap());
//     let r2 = u64::from_le_bytes(slot_hash[8..16].try_into().unwrap());
//     let r3 = u64::from_le_bytes(slot_hash[16..24].try_into().unwrap());
//     let r4 = u64::from_le_bytes(slot_hash[24..32].try_into().unwrap());
//     let r = r1 ^ r2 ^ r3 ^ r4;
//     // Returns a value in the range [0, 24] inclusive
//     r % 25
// }

#[allow(dead_code)]
async fn simulate_transaction(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
    instructions: &[solana_sdk::instruction::Instruction],
) {
    let blockhash = rpc.get_latest_blockhash().await.unwrap();
    let x = rpc
        .simulate_transaction(&Transaction::new_signed_with_payer(
            instructions,
            Some(&payer.pubkey()),
            &[payer],
            blockhash,
        ))
        .await;
    println!("Simulation result: {:?}", x);
}

#[allow(dead_code)]
async fn simulate_transaction_with_address_lookup_tables(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
    instructions: &[solana_sdk::instruction::Instruction],
    address_lookup_table_accounts: Vec<AddressLookupTableAccount>,
) {
    let blockhash = rpc.get_latest_blockhash().await.unwrap();
    let tx = VersionedTransaction {
        signatures: vec![Signature::default()],
        message: VersionedMessage::V0(
            SolanaMessage::try_compile(
                &payer.pubkey(),
                instructions,
                &address_lookup_table_accounts,
                blockhash,
            )
            .unwrap(),
        ),
    };
    let s = tx.sanitize();
    println!("Sanitize result: {:?}", s);
    s.unwrap();
    let x = rpc.simulate_transaction(&tx).await;
    println!("Simulation result: {:?}", x);
}

#[allow(unused)]
async fn submit_transaction_batches(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
    mut ixs: Vec<solana_sdk::instruction::Instruction>,
    batch_size: usize,
) -> Result<(), anyhow::Error> {
    // Batch and submit the instructions.
    while !ixs.is_empty() {
        let batch = ixs
            .drain(..std::cmp::min(batch_size, ixs.len()))
            .collect::<Vec<Instruction>>();
        submit_transaction_no_confirm(rpc, payer, &batch).await?;
    }
    Ok(())
}

#[allow(unused)]
async fn simulate_transaction_batches(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
    mut ixs: Vec<solana_sdk::instruction::Instruction>,
    batch_size: usize,
) -> Result<(), anyhow::Error> {
    // Batch and submit the instructions.
    while !ixs.is_empty() {
        let batch = ixs
            .drain(..std::cmp::min(batch_size, ixs.len()))
            .collect::<Vec<Instruction>>();
        simulate_transaction(rpc, payer, &batch).await;
    }
    Ok(())
}

async fn submit_transaction(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
    instructions: &[solana_sdk::instruction::Instruction],
) -> Result<solana_sdk::signature::Signature, anyhow::Error> {
    let blockhash = rpc.get_latest_blockhash().await?;
    let mut all_instructions = vec![
        ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
        ComputeBudgetInstruction::set_compute_unit_price(1_000_000),
    ];
    all_instructions.extend_from_slice(instructions);
    let transaction = Transaction::new_signed_with_payer(
        &all_instructions,
        Some(&payer.pubkey()),
        &[payer],
        blockhash,
    );

    match rpc
        .send_transaction_with_config(
            &transaction,
            RpcSendTransactionConfig {
                skip_preflight: true,
                ..RpcSendTransactionConfig::default()
            },
        )
        .await
    {
        Ok(signature) => {
            println!(
                "current time: {:?}, Transaction submitted: {:?}",
                std::time::Instant::now(),
                signature
            );
            Ok(signature)
        }
        Err(e) => {
            println!("Error submitting transaction: {:?}", e);
            Err(e.into())
        }
    }
}

async fn submit_transaction_no_confirm(
    rpc: &RpcClient,
    payer: &solana_sdk::signer::keypair::Keypair,
    instructions: &[solana_sdk::instruction::Instruction],
) -> Result<solana_sdk::signature::Signature, anyhow::Error> {
    let blockhash = rpc.get_latest_blockhash().await?;
    let mut all_instructions = vec![
        ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
        ComputeBudgetInstruction::set_compute_unit_price(1_000_000),
    ];
    all_instructions.extend_from_slice(instructions);
    let transaction = Transaction::new_signed_with_payer(
        &all_instructions,
        Some(&payer.pubkey()),
        &[payer],
        blockhash,
    );

    match rpc.send_transaction(&transaction).await {
        Ok(signature) => {
            println!("Transaction submitted: {:?}", signature);
            Ok(signature)
        }
        Err(e) => {
            println!("Error submitting transaction: {:?}", e);
            Err(e.into())
        }
    }
}

pub async fn get_program_accounts<T>(
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
    all_filters.extend(filters);
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
            let accounts = accounts
                .into_iter()
                .filter_map(|(pubkey, account)| {
                    if let Ok(account) = T::try_from_bytes(&account.data) {
                        Some((pubkey, account.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            Ok(accounts)
        }
        Err(err) => match err.kind {
            ClientErrorKind::Reqwest(err) => {
                if let Some(status_code) = err.status() {
                    if status_code == StatusCode::GONE {
                        panic!(
                                "\n{} Your RPC provider does not support the getProgramAccounts endpoint, needed to execute this command. Please use a different RPC provider.\n",
                                "ERROR"
                            );
                    }
                }
                return Err(anyhow::anyhow!("Failed to get program accounts: {}", err));
            }
            _ => return Err(anyhow::anyhow!("Failed to get program accounts: {}", err)),
        },
    }
}
