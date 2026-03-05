use base64::{Engine, prelude::BASE64_STANDARD};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey,
    pubkey::Pubkey,
    signer::Signer,
};
use spl_associated_token_account::get_associated_token_address_with_program_id;
use thiserror::Error;

pub const METAPLEX_METADATA_PROGRAM: Pubkey =
    pubkey!("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s");

pub const PUMPFUN: Pubkey = pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");
pub const EVENT_AUTHORITY: Pubkey = pubkey!("Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1");
pub const FEE_RECIPIENT: Pubkey = pubkey!("62qc2CNXwrYqQScmEdiZFFAnJR262PxWEuNQtxfafNgV");
pub const FEE_PROGRAM: Pubkey = pubkey!("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ");
pub const FEE_CONFIG: Pubkey = pubkey!("8Wf5TiAheLUqBrKXeYg2JtAFFMWtKdG2BSFgqUcPVwTt");
pub const MAYHEM_FEE_RECIPIENT: Pubkey = pubkey!("GesfTA3X2arioaHp8bbKdjG9vJtskViWACZoYvxp4twS");
pub const SYSTEM_PROGRAM: Pubkey = pubkey!("11111111111111111111111111111111");
pub const TOKEN_PROGRAM_2022: Pubkey = pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

const GLOBAL_SEED: &[u8] = b"global";
const BONDING_CURVE_SEED: &[u8] = b"bonding-curve";
const BONDING_CURVE_V2_SEED: &[u8] = b"bonding-curve-v2";
const CREATOR_VAULT_SEED: &[u8] = b"creator-vault";

pub const FEE_BASIS_POINTS: u64 = 95;
pub const CREATOR_FEE: u64 = 30;

pub fn get_global_pda() -> Pubkey {
    Pubkey::find_program_address(&[GLOBAL_SEED], &PUMPFUN).0
}

pub fn get_bonding_curve_pda(mint: &Pubkey) -> Option<Pubkey> {
    Pubkey::try_find_program_address(&[BONDING_CURVE_SEED, mint.as_ref()], &PUMPFUN)
        .map(|(pk, _)| pk)
}

pub fn get_creator_vault_pda(creator: &Pubkey) -> Option<Pubkey> {
    Pubkey::try_find_program_address(&[CREATOR_VAULT_SEED, creator.as_ref()], &PUMPFUN)
        .map(|(pk, _)| pk)
}

pub fn get_bonding_curve_v2_pda(mint: &Pubkey) -> Option<Pubkey> {
    Pubkey::try_find_program_address(&[BONDING_CURVE_V2_SEED, mint.as_ref()], &PUMPFUN)
        .map(|(pk, _)| pk)
}

pub fn get_user_vol_acc(user: &Pubkey) -> Option<Pubkey> {
    Pubkey::try_find_program_address(&[b"user_volume_accumulator", user.as_ref()], &PUMPFUN)
        .map(|(pk, _)| pk)
}

pub fn get_global_vol_acc() -> Option<Pubkey> {
    Pubkey::try_find_program_address(&[b"global_volume_accumulator"], &PUMPFUN).map(|(pk, _)| pk)
}

#[derive(Debug, Error)]
pub enum PumpFunError {
    #[error("failed to derive bonding curve PDA")]
    PdaDerivation,
    #[error("rpc error: {0}")]
    Rpc(#[from] solana_client::client_error::ClientError),
    #[error("borsh deserialize: {0}")]
    Deserialize(#[from] std::io::Error),
    #[error("curve is complete")]
    CurveComplete,
}

#[derive(Clone, Debug, BorshDeserialize)]
#[allow(dead_code)]
pub struct PumpFunCreateEvent {
    pub name: String,
    pub symbol: String,
    pub uri: String,
    pub mint: Pubkey,
    pub bonding_curve: Pubkey,
    pub user: Pubkey,
    pub creator: Pubkey,
    pub timestamp: i64,
    pub virtual_token_reserves: u64,
    pub virtual_sol_reserves: u64,
    pub real_token_reserves: u64,
    pub token_total_supply: u64,
    pub token_program: Pubkey,
    pub is_mayhem_mode: bool,
    pub is_cashback_enabled: bool,
}

#[derive(Clone, Debug, BorshDeserialize)]
#[allow(dead_code)]
pub struct PumpFunTradeEvent {
    pub mint: Pubkey,
    pub sol_amount: u64,
    pub token_amount: u64,
    pub is_buy: bool,
    pub user: Pubkey,
    pub timestamp: i64,
    pub virtual_sol_reserves: u64,
    pub virtual_token_reserves: u64,
    pub real_sol_reserves: u64,
    pub real_token_reserves: u64,
    pub fee_recipient: Pubkey,
    pub fee_basis_points: u64,
    pub fee: u64,
    pub creator: Pubkey,
    pub creator_fee_basis_points: u64,
    pub creator_fee: u64,
    pub track_volume: bool,
    pub total_unclaimed_tokens: u64,
    pub total_claimed_tokens: u64,
    pub current_sol_volume: u64,
    pub last_update_timestamp: i64,
    pub ix_name: String,
    pub mayhem_mode: bool,
    pub cashback_fee_basis_points: u64,
    pub cashback: u64,
}

const CREATE_DISCRIMINATOR: [u8; 8] = [27, 114, 169, 77, 222, 235, 99, 118];
const TRADE_DISCRIMINATOR: [u8; 8] = [189, 219, 127, 211, 78, 230, 97, 238];

#[derive(Clone, Debug)]
pub enum PumpFunEvent {
    Create(PumpFunCreateEvent),
    Trade(PumpFunTradeEvent),
}

fn decode_event(data: &[u8]) -> Option<PumpFunEvent> {
    if data.len() < 8 {
        return None;
    }
    match data[..8].try_into().ok()? {
        CREATE_DISCRIMINATOR => PumpFunCreateEvent::try_from_slice(&data[8..])
            .ok()
            .map(PumpFunEvent::Create),
        TRADE_DISCRIMINATOR => PumpFunTradeEvent::try_from_slice(&data[8..])
            .ok()
            .map(PumpFunEvent::Trade),
        _ => None,
    }
}

pub fn decode_events_from_logs(logs: &[String]) -> Vec<PumpFunEvent> {
    let mut events = Vec::new();
    for log in logs {
        if let Some(log_data) = log.strip_prefix("Program data: ") {
            if let Ok(data) = BASE64_STANDARD.decode(log_data) {
                if let Some(event) = decode_event(&data) {
                    events.push(event);
                }
            }
        }
    }
    events
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct BondingCurveAccount {
    pub discriminator: u64,
    pub virtual_token_reserves: u64,
    pub virtual_sol_reserves: u64,
    pub real_token_reserves: u64,
    pub real_sol_reserves: u64,
    pub token_total_supply: u64,
    pub complete: bool,
    pub creator: Pubkey,
    pub is_mayhem_mode: bool,
    pub is_cashback_coin: bool,
}

impl BondingCurveAccount {
    pub fn get_buy_token_amount_from_sol_amount(&self, amount: u64) -> u64 {
        if amount == 0 || self.virtual_token_reserves == 0 {
            return 0;
        }
        let total_fee_bps = FEE_BASIS_POINTS
            + if self.creator != Pubkey::default() {
                CREATOR_FEE
            } else {
                0
            };
        let amount_128 = amount as u128;
        let input_amount = amount_128
            .checked_mul(10_000)
            .unwrap()
            .checked_div(total_fee_bps as u128 + 10_000)
            .unwrap();
        let vtr = self.virtual_token_reserves as u128;
        let vsr = self.virtual_sol_reserves as u128;
        let rtr = self.real_token_reserves as u128;
        let denominator = vsr + input_amount;
        let tokens = input_amount.checked_mul(vtr).unwrap().checked_div(denominator).unwrap();
        tokens.min(rtr) as u64
    }

    pub fn get_sell_price(&self, amount: u64) -> Result<u64, PumpFunError> {
        if self.complete {
            return Err(PumpFunError::CurveComplete);
        }
        if amount == 0 {
            return Ok(0);
        }
        let total_fee_bps = FEE_BASIS_POINTS
            + if self.creator != Pubkey::default() {
                CREATOR_FEE
            } else {
                0
            };
        let sol_out = ((amount as u128) * (self.virtual_sol_reserves as u128))
            / ((self.virtual_token_reserves as u128) + (amount as u128));
        let fee = (sol_out * total_fee_bps as u128 + 10_000 - 1) / 10_000;
        Ok(sol_out.saturating_sub(fee) as u64)
    }
}

pub async fn fetch_bonding_curve(
    rpc: &RpcClient,
    mint: &Pubkey,
) -> Result<BondingCurveAccount, PumpFunError> {
    let pda = get_bonding_curve_pda(mint).ok_or(PumpFunError::PdaDerivation)?;
    let account = rpc.get_account(&pda).await?;
    let mut data = account.data.as_slice();
    let curve = BondingCurveAccount::deserialize(&mut data)?;
    Ok(curve)
}

#[derive(BorshSerialize, BorshDeserialize)]
pub struct Buy {
    pub amount: u64,
    pub max_sol_cost: u64,
}

impl Buy {
    const DISCRIMINATOR: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];

    pub fn data(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(24);
        data.extend_from_slice(&Self::DISCRIMINATOR);
        self.serialize(&mut data).unwrap();
        data
    }
}

#[derive(BorshSerialize, BorshDeserialize)]
pub struct Sell {
    pub amount: u64,
    pub min_sol_output: u64,
}

impl Sell {
    const DISCRIMINATOR: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];

    pub fn data(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(24);
        data.extend_from_slice(&Self::DISCRIMINATOR);
        self.serialize(&mut data).unwrap();
        data
    }
}

fn token_program_and_fee(is_mayhem: bool) -> (Pubkey, Pubkey) {
    if is_mayhem {
        (TOKEN_PROGRAM_2022, MAYHEM_FEE_RECIPIENT)
    } else {
        (TOKEN_PROGRAM_2022, FEE_RECIPIENT)
    }
}

fn get_ata(owner: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    get_associated_token_address_with_program_id(owner, mint, token_program)
}

pub fn build_buy_ix(
    payer: &impl Signer,
    mint: &Pubkey,
    creator: &Pubkey,
    is_mayhem_mode: bool,
    args: Buy,
) -> Instruction {
    let (token_prog, fee_recipient) = token_program_and_fee(is_mayhem_mode);
    let bonding_curve = get_bonding_curve_pda(mint).unwrap();
    let creator_vault = get_creator_vault_pda(creator).unwrap();
    let bonding_curve_v2 = get_bonding_curve_v2_pda(mint).unwrap();
    Instruction::new_with_bytes(
        PUMPFUN,
        &args.data(),
        vec![
            AccountMeta::new_readonly(get_global_pda(), false),
            AccountMeta::new(fee_recipient, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new(bonding_curve, false),
            AccountMeta::new(get_ata(&bonding_curve, mint, &token_prog), false),
            AccountMeta::new(get_ata(&payer.pubkey(), mint, &token_prog), false),
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
            AccountMeta::new_readonly(token_prog, false),
            AccountMeta::new(creator_vault, false),
            AccountMeta::new_readonly(EVENT_AUTHORITY, false),
            AccountMeta::new_readonly(PUMPFUN, false),
            AccountMeta::new(get_global_vol_acc().unwrap(), false),
            AccountMeta::new(get_user_vol_acc(&payer.pubkey()).unwrap(), false),
            AccountMeta::new_readonly(FEE_CONFIG, false),
            AccountMeta::new_readonly(FEE_PROGRAM, false),
            AccountMeta::new_readonly(bonding_curve_v2, false),
        ],
    )
}

#[derive(Clone, Debug)]
pub struct TokenMeta {
    pub name: String,
    pub symbol: String,
}

fn get_metadata_pda(mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[b"metadata", METAPLEX_METADATA_PROGRAM.as_ref(), mint.as_ref()],
        &METAPLEX_METADATA_PROGRAM,
    )
    .0
}

fn parse_metadata_fields(data: &[u8]) -> Option<TokenMeta> {
    if data.len() < 65 + 8 {
        return None;
    }
    let mut offset = 65;

    let name_len = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
    offset += 4;
    if offset + name_len > data.len() {
        return None;
    }
    let name = String::from_utf8_lossy(&data[offset..offset + name_len])
        .trim_end_matches('\0')
        .to_string();
    offset += name_len;

    if offset + 4 > data.len() {
        return None;
    }
    let symbol_len = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
    offset += 4;
    if offset + symbol_len > data.len() {
        return None;
    }
    let symbol = String::from_utf8_lossy(&data[offset..offset + symbol_len])
        .trim_end_matches('\0')
        .to_string();

    Some(TokenMeta { name, symbol })
}

pub async fn fetch_token_metadata(
    rpc: &RpcClient,
    mint: &Pubkey,
) -> Result<TokenMeta, PumpFunError> {
    let pda = get_metadata_pda(mint);
    let account = rpc.get_account(&pda).await?;
    parse_metadata_fields(&account.data).ok_or_else(|| {
        PumpFunError::Deserialize(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "failed to parse metadata",
        ))
    })
}

pub fn build_sell_ix(
    payer: &impl Signer,
    mint: &Pubkey,
    creator: &Pubkey,
    is_mayhem_mode: bool,
    args: Sell,
) -> Instruction {
    let (token_prog, fee_recipient) = token_program_and_fee(is_mayhem_mode);
    let bonding_curve = get_bonding_curve_pda(mint).unwrap();
    let creator_vault = get_creator_vault_pda(creator).unwrap();
    let bonding_curve_v2 = get_bonding_curve_v2_pda(mint).unwrap();
    Instruction::new_with_bytes(
        PUMPFUN,
        &args.data(),
        vec![
            AccountMeta::new_readonly(get_global_pda(), false),
            AccountMeta::new(fee_recipient, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new(bonding_curve, false),
            AccountMeta::new(get_ata(&bonding_curve, mint, &token_prog), false),
            AccountMeta::new(get_ata(&payer.pubkey(), mint, &token_prog), false),
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
            AccountMeta::new(creator_vault, false),
            AccountMeta::new_readonly(token_prog, false),
            AccountMeta::new_readonly(EVENT_AUTHORITY, false),
            AccountMeta::new_readonly(PUMPFUN, false),
            AccountMeta::new_readonly(FEE_CONFIG, false),
            AccountMeta::new_readonly(FEE_PROGRAM, false),
            AccountMeta::new_readonly(bonding_curve_v2, false),
        ],
    )
}
