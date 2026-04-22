use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::agent::AgentId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionEntry {
    pub round: u64,
    pub agent_id: AgentId,
    pub trader_type: String,
    pub action_json: String,
    pub price: f64,
    pub timestamp_ms: i64,
}

pub struct ActionLog {
    buffer: VecDeque<ActionEntry>,
    db_path: String,
    batch_size: usize,
    total_written: u64,
    flush_tx: Option<mpsc::UnboundedSender<Vec<ActionEntry>>>,
}

impl ActionLog {
    pub fn new(db_path: &str, batch_size: usize) -> Self {
        Self {
            buffer: VecDeque::new(),
            db_path: db_path.to_string(),
            batch_size,
            total_written: 0,
            flush_tx: None,
        }
    }

    pub async fn with_background_writer(db_path: &str, batch_size: usize) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<ActionEntry>>();
        let path = db_path.to_string();

        tokio::spawn(async move {
            info!("ActionLog background writer started for {}", path);
            while let Some(batch) = rx.recv().await {
                if let Err(e) = write_jsonl_batch(&path, &batch) {
                    error!("ActionLog write error: {}", e);
                }
            }
            info!("ActionLog background writer stopped");
        });

        Self {
            buffer: VecDeque::new(),
            db_path: db_path.to_string(),
            batch_size,
            total_written: 0,
            flush_tx: Some(tx),
        }
    }

    pub fn push_batch(&mut self, entries: Vec<ActionEntry>) {
        for e in entries {
            self.buffer.push_back(e);
        }
    }

    pub async fn flush_if_ready(&mut self) {
        if self.buffer.len() >= self.batch_size {
            self.flush().await;
        }
    }

    pub async fn flush(&mut self) {
        if self.buffer.is_empty() {
            return;
        }

        let batch: Vec<ActionEntry> = self.buffer.drain(..).collect();
        let count = batch.len();

        if let Some(tx) = &self.flush_tx {
            if let Err(e) = tx.send(batch) {
                warn!("ActionLog flush channel closed: {}", e);
            }
        } else {
            if let Err(e) = write_jsonl_batch(&self.db_path, &batch) {
                error!("ActionLog sync write error: {}", e);
            }
        }

        self.total_written += count as u64;
    }

    pub fn total_written(&self) -> u64 {
        self.total_written
    }
    pub fn buffered(&self) -> usize {
        self.buffer.len()
    }
}

#[derive(Debug)]
pub struct ActionQuery;

impl ActionQuery {
    pub fn buy_sell_ratio(
        entries: &[ActionEntry],
        last_n_rounds: u64,
        current_round: u64,
    ) -> (usize, usize) {
        let min_round = current_round.saturating_sub(last_n_rounds);
        let buys = entries
            .iter()
            .filter(|e| e.round >= min_round && e.action_json.contains("\"Buy\""))
            .count();
        let sells = entries
            .iter()
            .filter(|e| e.round >= min_round && e.action_json.contains("\"Sell\""))
            .count();
        (buys, sells)
    }

    pub fn most_active_agent(
        entries: &[ActionEntry],
        last_n_rounds: u64,
        current_round: u64,
    ) -> Option<AgentId> {
        use std::collections::HashMap;
        let min_round = current_round.saturating_sub(last_n_rounds);

        let mut counts: HashMap<AgentId, usize> = HashMap::new();
        for e in entries.iter().filter(|e| e.round >= min_round) {
            *counts.entry(e.agent_id).or_default() += 1;
        }

        counts.into_iter().max_by_key(|(_, c)| *c).map(|(id, _)| id)
    }
}

fn write_jsonl_batch(path: &str, batch: &[ActionEntry]) -> std::io::Result<()> {
    use std::fs::OpenOptions;
    use std::io::Write;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    for entry in batch {
        let line = serde_json::to_string(entry)?;
        writeln!(file, "{}", line)?;
    }
    Ok(())
}
