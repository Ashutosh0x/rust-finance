#![forbid(unsafe_code)]
use reqwest::Client;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Clone, Debug)]
pub struct NodeSelector {
    client: Client,
    candidates: Arc<RwLock<Vec<String>>>,
    best: Arc<RwLock<Option<String>>>,
}

impl NodeSelector {
    pub fn new(nodes: Vec<String>) -> Self {
        Self {
            client: Client::builder().timeout(Duration::from_millis(600)).build().unwrap_or_default(),
            candidates: Arc::new(RwLock::new(nodes)),
            best: Arc::new(RwLock::new(None)),
        }
    }

    /// Start background task measuring latency every `interval`
    pub fn start(self: Arc<Self>, interval: Duration) {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                if let Err(e) = self.update_once().await {
                    tracing::warn!("node selector measurement failed: {:?}", e);
                }
            }
        });
    }

    pub async fn update_once(&self) -> anyhow::Result<()> {
        let nodes = { self.candidates.read().await.clone() };
        
        let mut tasks = Vec::new();
        for url in nodes {
            let client = self.client.clone();
            tasks.push(tokio::spawn(async move {
                let ms = measure_rpc_latency(&client, &url).await;
                (url, ms)
            }));
        }

        let results = futures::future::join_all(tasks).await;
        let mut latencies: Vec<(String, u128)> = Vec::new();

        for res in results {
            if let Ok((url, Some(ms))) = res {
                latencies.push((url, ms));
            }
        }

        latencies.sort_by_key(|(_, d)| *d);
        if let Some((best, _)) = latencies.first() {
            let mut guard = self.best.write().await;
            *guard = Some(best.clone());
            tracing::info!("node-selector picked best node: {} ({}ms)", best, latencies.first().map(|x| x.1).unwrap_or(0));
        } else {
            tracing::warn!("node-selector: no healthy nodes");
        }
        Ok(())
    }

    pub async fn get_best(&self) -> String {
        self.best.read().await.clone().unwrap_or_else(|| {
             // Fallback to first candidate if nothing picked yet
             "https://api.mainnet-beta.solana.com".to_string()
        })
    }

    pub async fn set_nodes(&self, nodes: Vec<String>) {
         let mut guard = self.candidates.write().await;
         *guard = nodes;
    }
}

async fn measure_rpc_latency(client: &Client, url: &str) -> Option<u128> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getSlot",
        "params": []
    });

    let start = Instant::now();
    let res = client.post(url).json(&body).send().await;
    match res {
        Ok(_) => Some(start.elapsed().as_millis()),
        Err(_) => None,
    }
}
