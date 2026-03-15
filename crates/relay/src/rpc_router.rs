// crates/relay/src/rpc_router.rs
//
// Solana RPC node router.
// Continuously benchmarks configured RPC nodes (Helius, Triton, QuickNode)
// and routes each transaction through the lowest-latency healthy node.
// Falls back to secondary/tertiary nodes automatically on failure.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time::{interval, sleep};
use tracing::{debug, error, info, warn};

/// A configured RPC endpoint.
#[derive(Debug, Clone)]
pub struct RpcNode {
    pub name: &'static str,
    pub url: String,
    pub ws_url: Option<String>,
    pub priority: u8, // lower = higher priority when latencies are equal
}

/// Runtime health & latency tracking for one node.
#[derive(Debug, Clone)]
pub struct NodeStats {
    pub name: &'static str,
    /// Exponential moving average of round-trip latency.
    pub latency_ema_us: f64,
    pub consecutive_failures: u32,
    pub is_healthy: bool,
    pub last_probe: Option<Instant>,
    pub total_requests: u64,
    pub total_failures: u64,
}

impl NodeStats {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            latency_ema_us: f64::MAX,
            consecutive_failures: 0,
            is_healthy: true,
            last_probe: None,
            total_requests: 0,
            total_failures: 0,
        }
    }

    fn update_latency(&mut self, sample_us: u64) {
        const ALPHA: f64 = 0.2; // EMA smoothing factor
        if self.latency_ema_us == f64::MAX {
            self.latency_ema_us = sample_us as f64;
        } else {
            self.latency_ema_us = ALPHA * sample_us as f64 + (1.0 - ALPHA) * self.latency_ema_us;
        }
        self.consecutive_failures = 0;
        self.is_healthy = true;
        self.last_probe = Some(Instant::now());
    }

    fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        self.total_failures += 1;
        if self.consecutive_failures >= 3 {
            if self.is_healthy {
                warn!(
                    node = self.name,
                    failures = self.consecutive_failures,
                    "RPC node marked unhealthy"
                );
            }
            self.is_healthy = false;
        }
    }
}

/// Router configuration.
#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// Probe interval for latency benchmarking.
    pub probe_interval: Duration,
    /// Timeout for a single probe request.
    pub probe_timeout: Duration,
    /// How long an unhealthy node must recover before re-entering rotation.
    pub recovery_wait: Duration,
    /// Maximum failures before a node is considered dead.
    pub max_failures: u32,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            probe_interval: Duration::from_secs(5),
            probe_timeout: Duration::from_millis(500),
            recovery_wait: Duration::from_secs(30),
            max_failures: 3,
        }
    }
}

type StatsMap = Arc<RwLock<HashMap<String, NodeStats>>>;

/// The RPC router — clone cheaply, share across tasks.
#[derive(Clone)]
pub struct RpcRouter {
    nodes: Vec<RpcNode>,
    stats: StatsMap,
    cfg: RouterConfig,
    http: reqwest::Client,
}

impl RpcRouter {
    /// Build from a list of nodes and start background probing.
    pub fn new(nodes: Vec<RpcNode>, cfg: RouterConfig) -> Self {
        let mut stats_map = HashMap::new();
        for node in &nodes {
            stats_map.insert(node.url.clone(), NodeStats::new(node.name));
        }

        let router = Self {
            nodes,
            stats: Arc::new(RwLock::new(stats_map)),
            cfg,
            http: reqwest::Client::builder()
                .timeout(Duration::from_millis(2000))
                .build()
                .expect("HTTP client"),
        };

        // Spawn background prober
        let prober = router.clone();
        tokio::spawn(async move {
            prober.probe_loop().await;
        });

        router
    }

    /// Convenience constructor with standard mainnet nodes.
    pub fn mainnet(helius_key: &str, quicknode_url: &str, triton_url: &str) -> Self {
        let nodes = vec![
            RpcNode {
                name: "helius",
                url: format!("https://mainnet.helius-rpc.com/?api-key={helius_key}"),
                ws_url: Some(format!("wss://mainnet.helius-rpc.com/?api-key={helius_key}")),
                priority: 1,
            },
            RpcNode {
                name: "triton",
                url: triton_url.to_string(),
                ws_url: None,
                priority: 2,
            },
            RpcNode {
                name: "quicknode",
                url: quicknode_url.to_string(),
                ws_url: None,
                priority: 3,
            },
        ];
        Self::new(nodes, RouterConfig::default())
    }

    /// Select the best available node URL for a transaction.
    ///
    /// Returns `None` only if ALL nodes are unhealthy.
    pub async fn best_node(&self) -> Option<&RpcNode> {
        let stats = self.stats.read().await;

        let mut candidates: Vec<(&RpcNode, &NodeStats)> = self
            .nodes
            .iter()
            .filter_map(|n| {
                let s = stats.get(&n.url)?;
                if s.is_healthy { Some((n, s)) } else { None }
            })
            .collect();

        if candidates.is_empty() {
            // All nodes unhealthy — try lowest-failure-count as last resort
            warn!("All RPC nodes unhealthy — using best-effort fallback");
            candidates = self
                .nodes
                .iter()
                .filter_map(|n| stats.get(&n.url).map(|s| (n, s)))
                .collect();
        }

        // Sort by EMA latency, break ties by priority
        candidates.sort_by(|a, b| {
            a.1.latency_ema_us
                .partial_cmp(&b.1.latency_ema_us)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.priority.cmp(&b.0.priority))
        });

        candidates.first().map(|(node, _)| *node)
    }

    /// Send a JSON-RPC request through the best available node.
    /// Falls back to secondary nodes on failure.
    pub async fn send_rpc(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RpcError> {
        let ordered_nodes = self.nodes_by_latency().await;

        for node in &ordered_nodes {
            let result = self.try_rpc(node, method, params.clone()).await;
            match result {
                Ok(resp) => {
                    debug!(node = node.name, method, "RPC call succeeded");
                    return Ok(resp);
                }
                Err(e) => {
                    warn!(node = node.name, method, error = %e, "RPC call failed — trying next");
                    self.record_failure(&node.url).await;
                }
            }
        }

        Err(RpcError::AllNodesFailed)
    }

    async fn try_rpc(
        &self,
        node: &RpcNode,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RpcError> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });

        let start = Instant::now();
        let resp = self
            .http
            .post(&node.url)
            .json(&body)
            .timeout(self.cfg.probe_timeout)
            .send()
            .await
            .map_err(|e| RpcError::Http(e.to_string()))?;

        let elapsed_us = start.elapsed().as_micros() as u64;
        let json: serde_json::Value = resp.json().await.map_err(|e| RpcError::Http(e.to_string()))?;

        // Update latency stats
        {
            let mut stats = self.stats.write().await;
            if let Some(s) = stats.get_mut(&node.url) {
                s.update_latency(elapsed_us);
                s.total_requests += 1;
            }
        }

        if let Some(err) = json.get("error") {
            return Err(RpcError::RpcError(err.to_string()));
        }

        json.get("result")
            .cloned()
            .ok_or(RpcError::MissingResult)
    }

    async fn nodes_by_latency(&self) -> Vec<&RpcNode> {
        let stats = self.stats.read().await;
        let mut nodes: Vec<&RpcNode> = self.nodes.iter().collect();
        nodes.sort_by(|a, b| {
            let la = stats.get(&a.url).map(|s| s.latency_ema_us).unwrap_or(f64::MAX);
            let lb = stats.get(&b.url).map(|s| s.latency_ema_us).unwrap_or(f64::MAX);
            la.partial_cmp(&lb).unwrap_or(std::cmp::Ordering::Equal)
        });
        nodes
    }

    async fn record_failure(&self, url: &str) {
        let mut stats = self.stats.write().await;
        if let Some(s) = stats.get_mut(url) {
            s.record_failure();
        }
    }

    /// Continuous background loop: probe all nodes every `probe_interval`.
    async fn probe_loop(&self) {
        let mut ticker = interval(self.cfg.probe_interval);
        loop {
            ticker.tick().await;
            for node in &self.nodes {
                let _ = self
                    .try_rpc(node, "getHealth", serde_json::Value::Null)
                    .await;
            }

            // Log current routing table
            let stats = self.stats.read().await;
            for (url, s) in stats.iter() {
                debug!(
                    node = s.name,
                    latency_us = format!("{:.0}", s.latency_ema_us),
                    healthy = s.is_healthy,
                    "RPC node status"
                );
            }
        }
    }

    pub async fn stats_snapshot(&self) -> HashMap<String, NodeStats> {
        self.stats.read().await.clone()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("JSON-RPC error: {0}")]
    RpcError(String),
    #[error("Missing result field in response")]
    MissingResult,
    #[error("All RPC nodes failed")]
    AllNodesFailed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_latency_ema_updates() {
        let mut stats = NodeStats::new("test");
        stats.update_latency(1000);
        assert_eq!(stats.latency_ema_us, 1000.0);
        stats.update_latency(500);
        // EMA: 0.2 * 500 + 0.8 * 1000 = 900
        assert!((stats.latency_ema_us - 900.0).abs() < 1.0);
    }

    #[test]
    fn test_node_marked_unhealthy_after_3_failures() {
        let mut stats = NodeStats::new("test");
        stats.update_latency(100); // mark healthy first
        for _ in 0..3 {
            stats.record_failure();
        }
        assert!(!stats.is_healthy);
        assert_eq!(stats.consecutive_failures, 3);
    }
}
