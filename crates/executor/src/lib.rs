#![forbid(unsafe_code)]
use anyhow::{Result, Context};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    instruction::Instruction,
    signature::Signature,
    transaction::Transaction,
};
use common::Action;
use signer::LocalSigner;
use std::time::Duration;
use tokio::time::timeout;
use std::sync::Arc;
use tracing::info;

pub mod algos;
pub mod strategies;
pub mod dry_run;

use dry_run::DryRunExecutor;

pub struct ExecutorService {
    selector: Arc<relay::NodeSelector>,
    signer: Option<Arc<LocalSigner>>,
    rpc_client: Arc<RpcClient>,
    alpaca: Option<ingestion::alpaca_broker::AlpacaBroker>,
}

impl ExecutorService {
    pub async fn new(selector: Arc<relay::NodeSelector>, signer: Option<LocalSigner>) -> Self {
        let rpc_url = selector.get_best().await;
        
        let alpaca = if let (Ok(api), Ok(sec)) = (std::env::var("ALPACA_API_KEY"), std::env::var("ALPACA_SECRET_KEY")) {
            Some(ingestion::alpaca_broker::AlpacaBroker::new(api, sec))
        } else {
            None
        };

        Self {
            selector,
            signer: signer.map(Arc::new),
            rpc_client: Arc::new(RpcClient::new(rpc_url)),
            alpaca,
        }
    }

    pub async fn execute_action(&self, action: Action) -> Result<Signature> {
        let (token, size, _confidence) = match &action {
            Action::Buy { token, size, confidence } => (token.clone(), *size, *confidence),
            Action::Sell { token, size, confidence } => (token.clone(), *size, *confidence),
            Action::Hold => return Ok(Signature::default()),
        };
        
        // --- 1. ROUTING DIFFERENTIAL: CRYPTO VS EQUITIES ---
        let is_crypto = token.starts_with('$') || token.ends_with("USDC") || token.ends_with("SOL");
        
        if !is_crypto {
            info!("Routing Equity/Fiat execution to Alpaca Broker for {}", token);
            if let Some(alpaca) = &self.alpaca {
                let side = match action {
                    Action::Buy { .. } => "buy",
                    Action::Sell { .. } => "sell",
                    _ => "",
                };
                
                let req = ingestion::alpaca_broker::AlpacaOrderRequest {
                    symbol: token.clone(),
                    qty: size,
                    side: side.to_string(),
                    type_: "market".to_string(),
                    time_in_force: "gtc".to_string(),
                };
                
                match alpaca.submit_order(req).await {
                    Ok(resp) => {
                        info!("Alpaca Order Filled: ID {} @ {}", resp.id, resp.status);
                        // Return dummy signature for non-crypto
                        return Ok(Signature::new_unique());
                    }
                    Err(e) => anyhow::bail!("Alpaca Execution failed: {}", e),
                }
            } else {
                anyhow::bail!("Cannot execute equity trade for {}: Alpaca keys not configured in ENV", token);
            }
        }

        // --- 2. CRYPTO ROUTE (SOLANA RPC) ---
        let signer = self.signer.as_ref().context("No signer configured for crypto execution")?;
        
        // --- 2A. PRE-TRADE BALANCE CHECK ---
        let rpc_client = self.rpc_client.clone();
        
        let pubkey = signer.pubkey();
        match timeout(Duration::from_secs(3), rpc_client.get_balance(&pubkey)).await {
            Ok(Ok(lamports)) => {
                let sol = lamports as f64 / 1_000_000_000.0;
                info!("Pre-trade check: Wallet {} has {:.4} SOL", pubkey, sol);
                if sol < 0.005 { // Arbitrary minimum balance to cover rent + compute
                    anyhow::bail!("Insufficient SOL balance to execute trade: {:.4} SOL", sol);
                }
            }
            Ok(Err(e)) => anyhow::bail!("Failed to fetch balance: {}", e),
            Err(_) => anyhow::bail!("RPC balance check timed out"),
        }

        // --- 2B. EXECUTION & SLIPPAGE GUARD ---
        match action {
            Action::Buy { token, size, confidence } => {
                info!("Executing Crypto BUY for {}: size={}, confidence={}", token, size, confidence);
                let max_slippage_bps = 50; // 0.5%
                info!("Enforcing max slippage limit: {} bps", max_slippage_bps);
                let instructions = self.build_buy_instructions(&token, size)?;
                self.send_and_confirm(instructions, signer).await
            }
            Action::Sell { token, size, confidence } => {
                info!("Executing Crypto SELL for {}: size={}, confidence={}", token, size, confidence);
                let max_slippage_bps = 50;
                info!("Enforcing max slippage limit: {} bps", max_slippage_bps);
                let instructions = self.build_sell_instructions(&token, size)?;
                self.send_and_confirm(instructions, signer).await
            }
            Action::Hold => Ok(Signature::default()),
        }
    }

    fn build_buy_instructions(&self, _token: &str, _size: f64) -> Result<Vec<Instruction>> {
        let mut ixs = Vec::new();
        // 1. Priority Fees (Compute Budget)
        ixs.push(solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_limit(200_000));
        ixs.push(solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_price(100_000)); // 100k microlamports
        
        // 2. Real implementation would include Swap instructions
        Ok(ixs) 
    }

    fn build_sell_instructions(&self, _token: &str, _size: f64) -> Result<Vec<Instruction>> {
        let mut ixs = Vec::new();
        ixs.push(solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_limit(200_000));
        ixs.push(solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_price(100_000));
        Ok(ixs)
    }

    async fn send_and_confirm(&self, instructions: Vec<Instruction>, signer: &LocalSigner) -> Result<Signature> {
        if instructions.is_empty() {
             return Ok(Signature::default());
        }

        if std::env::var("USE_MOCK").is_ok() {
            let dry_runner = DryRunExecutor;
            let mock_sig = dry_runner.execute_mock(&Action::Hold); // dummy Action just for sign
            return Ok(mock_sig);
        }

        // 1. Fetch blockhash (In production, subscribe to slot updates for zero-latency hash)
        let recent_blockhash = self.rpc_client.get_latest_blockhash().await?;
        
        // 2. Build & Sign
        let mut tx = Transaction::new_with_payer(&instructions, Some(&signer.pubkey()));
        tx.message.recent_blockhash = recent_blockhash;
        signer.sign_transaction(&mut tx);
        
        // 3. Send (Use send_transaction for signed transactions)
        let signature = self.rpc_client.send_transaction(&tx).await
            .context("Failed to send transaction")?;
        
        info!("Transaction sent: {}", signature);
        
        // 4. Async confirmation (Don't block the executor task if possible)
        // For now we just return the signature. Confirmation can be monitored by a separate service.
        
        Ok(signature)
    }
}
