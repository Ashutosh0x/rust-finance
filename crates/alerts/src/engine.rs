// crates/alerts/src/engine.rs
// Real-time asynchronous notification routing
// Dispatch critical events to Telegram, Discord, or generic Webhooks

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::mpsc::{Receiver, Sender};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub severity: AlertSeverity,
    pub title: String,
    pub message: String,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct AlertConfig {
    pub telegram_bot_token: Option<String>,
    pub telegram_chat_id: Option<String>,
    pub discord_webhook_url: Option<String>,
    pub minimum_severity: AlertSeverity,
}

pub struct AlertEngine {
    config: AlertConfig,
    client: Client,
    rx: Receiver<Alert>,
}

impl AlertEngine {
    pub fn new(config: AlertConfig, rx: Receiver<Alert>) -> Self {
        Self {
            config,
            client: Client::new(),
            rx,
        }
    }

    /// Run the alerting loop. Spawn this in a background tokio task.
    pub async fn run(mut self) {
        tracing::info!("Alert engine started");
        
        while let Some(alert) = self.rx.recv().await {
            if self.should_send(&alert) {
                // Dispatch concurrently so a slow Discord webhook doesn't block Telegram
                let telegram_fut = self.send_telegram(alert.clone());
                let discord_fut = self.send_discord(alert.clone());
                
                tokio::join!(telegram_fut, discord_fut);
            }
        }
        
        tracing::warn!("Alert engine shutting down (channel closed)");
    }

    fn should_send(&self, alert: &Alert) -> bool {
        match (&self.config.minimum_severity, &alert.severity) {
            (AlertSeverity::Critical, AlertSeverity::Critical) => true,
            (AlertSeverity::Warning, AlertSeverity::Warning) | (AlertSeverity::Warning, AlertSeverity::Critical) => true,
            (AlertSeverity::Info, _) => true,
            _ => false,
        }
    }

    async fn send_telegram(&self, alert: Alert) {
        let (token, chat_id) = match (&self.config.telegram_bot_token, &self.config.telegram_chat_id) {
            (Some(t), Some(c)) => (t, c),
            _ => return,
        };

        let emoji = match alert.severity {
            AlertSeverity::Info => "ℹ️",
            AlertSeverity::Warning => "⚠️",
            AlertSeverity::Critical => "🚨",
        };

        let text = format!("{} *{}*\n_{}_\n\n{}", emoji, alert.title, alert.source, alert.message);
        let url = format!("https://api.telegram.org/bot{}/sendMessage", token);

        let mut body = HashMap::new();
        body.insert("chat_id", chat_id.clone());
        body.insert("text", text);
        body.insert("parse_mode", "Markdown".to_string());

        match self.client.post(&url).json(&body).send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::debug!("Telegram alert sent: {}", alert.title);
            }
            Ok(resp) => {
                tracing::error!("Telegram alert failed with status: {}", resp.status());
            }
            Err(e) => {
                tracing::error!("Telegram alert network error: {}", e);
            }
        }
    }

    async fn send_discord(&self, alert: Alert) {
        let webhook_url = match &self.config.discord_webhook_url {
            Some(url) => url,
            None => return,
        };

        let color = match alert.severity {
            AlertSeverity::Info => 3447003,      // Blue
            AlertSeverity::Warning => 16776960,  // Yellow
            AlertSeverity::Critical => 15158332, // Red
        };

        let payload = serde_json::json!({
            "embeds": [{
                "title": alert.title,
                "description": alert.message,
                "color": color,
                "footer": {
                    "text": alert.source
                }
            }]
        });

        match self.client.post(webhook_url).json(&payload).send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::debug!("Discord alert sent: {}", alert.title);
            }
            Ok(resp) => {
                tracing::error!("Discord alert failed with status: {}", resp.status());
            }
            Err(e) => {
                tracing::error!("Discord alert network error: {}", e);
            }
        }
    }
}

/// Convenience struct to hold a Sender to the alert engine
#[derive(Clone)]
pub struct Alerter {
    tx: Sender<Alert>,
}

impl Alerter {
    pub fn new(tx: Sender<Alert>) -> Self {
        Self { tx }
    }

    pub async fn info(&self, source: &str, title: &str, message: &str) {
        let _ = self.tx.send(Alert {
            severity: AlertSeverity::Info,
            title: title.to_string(),
            message: message.to_string(),
            source: source.to_string(),
        }).await;
    }

    pub async fn warn(&self, source: &str, title: &str, message: &str) {
        let _ = self.tx.send(Alert {
            severity: AlertSeverity::Warning,
            title: title.to_string(),
            message: message.to_string(),
            source: source.to_string(),
        }).await;
    }

    pub async fn critical(&self, source: &str, title: &str, message: &str) {
        let _ = self.tx.send(Alert {
            severity: AlertSeverity::Critical,
            title: title.to_string(),
            message: message.to_string(),
            source: source.to_string(),
        }).await;
    }
}
