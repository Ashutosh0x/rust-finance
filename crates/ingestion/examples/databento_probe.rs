//! Determine whether this account is entitled to a Databento live dataset.
//!
//! Authenticates and closes. It never calls `start()` and never subscribes, so
//! no records are delivered and nothing is billed — the question being asked is
//! only "would the gateway accept this session", and that is answered during
//! the CRAM handshake in `build()`.
//!
//! Run with:
//! ```text
//! DATABENTO_API_KEY=... cargo run -p ingestion --features databento \
//!   --example databento_probe -- XNAS.ITCH GLBX.MDP3
//! ```

use std::time::Instant;

use databento::LiveClient;

#[tokio::main]
async fn main() {
    let Ok(key) = std::env::var("DATABENTO_API_KEY") else {
        eprintln!("DATABENTO_API_KEY not set");
        std::process::exit(2);
    };

    let datasets: Vec<String> = std::env::args().skip(1).collect();
    if datasets.is_empty() {
        eprintln!("usage: databento_probe <DATASET> [DATASET...]");
        std::process::exit(2);
    }

    println!("{:<16} {:>10}  result", "dataset", "auth");
    println!("{}", "-".repeat(72));

    for dataset in datasets {
        let started = Instant::now();
        let outcome = LiveClient::builder()
            .key(&key)
            .expect("key rejected locally")
            .dataset(dataset.clone())
            .build()
            .await;

        let elapsed = started.elapsed().as_millis();
        match outcome {
            Ok(mut client) => {
                // Closed straight away: authenticated is all we needed to know.
                let _ = client.close().await;
                println!("{dataset:<16} {elapsed:>8}ms  ENTITLED (session accepted, closed)");
            }
            Err(e) => {
                println!("{dataset:<16} {elapsed:>8}ms  REFUSED: {e}");
            }
        }
    }
}
