use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Heartbeat {
    Ping,
    Pong,
}
