use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    compute_budget::ComputeBudgetInstruction,
    signature::{Keypair, Signature},
    signer::Signer,
    transaction::Transaction,
};
use spl_associated_token_account::instruction::create_associated_token_account_idempotent;
use tracing::{info, warn};
use crate::pumpfun::{self, Buy, Sell};
use crate::send_transaction::{self, SolamiSender, build_tip_ix};

pub async fn buy_token(
    rpc: &RpcClient,
    sender: Option<&SolamiSender>,
    payer: &Keypair,
    mint: &solana_sdk::pubkey::Pubkey,
    creator: &solana_sdk::pubkey::Pubkey,
    buy_amount_lamports: u64,
    slippage_bps: u64,
    priority_fee_lamports: u64,
    tip_amount_lamports: u64,
) -> Result<(Signature, u64, u64), Box<dyn std::error::Error + Send + Sync>> {
    let mut curve = None;
    for attempt in 0..3 {
        match pumpfun::fetch_bonding_curve(rpc, mint).await {
            Ok(c) => {
                curve = Some(c);
                break;
            }
            Err(e) => {
                warn!(attempt, error = %e, "fetch bonding curve failed, retrying");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
    let curve = curve.ok_or("failed to fetch bonding curve after 3 retries")?;

    let raw_token_amount = curve.get_buy_token_amount_from_sol_amount(buy_amount_lamports);
    if raw_token_amount == 0 {
        return Err("calculated 0 tokens for buy".into());
    }
    let token_amount = raw_token_amount.saturating_sub(raw_token_amount * slippage_bps / 10_000);
    let actual_sol_cost = curve.get_buy_cost_for_tokens(token_amount);
    let max_sol_cost = buy_amount_lamports + (buy_amount_lamports * slippage_bps / 10_000);

    let create_ata_ix = create_associated_token_account_idempotent(
        &payer.pubkey(),
        &payer.pubkey(),
        mint,
        &pumpfun::TOKEN_PROGRAM_2022,
    );

    let buy_ix = pumpfun::build_buy_ix(
        payer,
        mint,
        creator,
        curve.is_mayhem_mode,
        Buy {
            amount: token_amount,
            max_sol_cost,
        },
    );

    let mut ixs = vec![
        ComputeBudgetInstruction::set_compute_unit_limit(200_000),
        ComputeBudgetInstruction::set_compute_unit_price(priority_fee_lamports),
        create_ata_ix,
        buy_ix,
    ];
    if sender.is_some() && tip_amount_lamports > 0 {
        ixs.push(build_tip_ix(&payer.pubkey(), tip_amount_lamports));
    }

    let recent_blockhash = rpc.get_latest_blockhash().await?;
    let tx = Transaction::new_signed_with_payer(&ixs, Some(&payer.pubkey()), &[payer], recent_blockhash);

    let sig = match sender {
        Some(s) => s.send_transaction(&tx).await?,
        None => {
            info!("Sending via rpc");
            send_transaction::send_transaction_rpc(rpc, &tx).await?},
    };

    info!(%sig, %mint, token_amount, actual_sol_cost, "buy tx sent");
    Ok((sig, token_amount, actual_sol_cost))
}

pub async fn sell_token(
    rpc: &RpcClient,
    sender: Option<&SolamiSender>,
    payer: &Keypair,
    mint: &solana_sdk::pubkey::Pubkey,
    creator: &solana_sdk::pubkey::Pubkey,
    token_amount: u64,
    slippage_bps: u64,
    priority_fee_lamports: u64,
    tip_amount_lamports: u64,
) -> Result<Signature, Box<dyn std::error::Error + Send + Sync>> {
    let curve = pumpfun::fetch_bonding_curve(rpc, mint).await?;

    let expected_sol = curve.get_sell_price(token_amount)?;
    let min_sol_output = expected_sol.saturating_sub(expected_sol * slippage_bps / 10_000);

    let sell_ix = pumpfun::build_sell_ix(
        payer,
        mint,
        creator,
        curve.is_mayhem_mode,
        Sell {
            amount: token_amount,
            min_sol_output,
        },
    );

    let mut ixs = vec![
        ComputeBudgetInstruction::set_compute_unit_limit(200_000),
        ComputeBudgetInstruction::set_compute_unit_price(priority_fee_lamports),
        sell_ix,
    ];
    if sender.is_some() && tip_amount_lamports > 0 {
        ixs.push(build_tip_ix(&payer.pubkey(), tip_amount_lamports));
    }

    let recent_blockhash = rpc.get_latest_blockhash().await?;
    let tx = Transaction::new_signed_with_payer(&ixs, Some(&payer.pubkey()), &[payer], recent_blockhash);

    let sig = match sender {
        Some(s) => s.send_transaction(&tx).await?,
        None => send_transaction::send_transaction_rpc(rpc, &tx).await?,
    };

    info!(%sig, %mint, token_amount, "sell tx sent");
    Ok(sig)
}
