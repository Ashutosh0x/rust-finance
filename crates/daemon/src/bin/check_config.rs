// crates/daemon/src/bin/check_config.rs
use common::config::AppConfig;
use dotenvy::dotenv;

fn main() {
    dotenv().ok();

    println!("┌─────────────────────────────────────┐");
    println!("│   RustForge Configuration Checker   │");
    println!("├─────────────────────────────────────┤");

    let checks = vec![
        ("FINNHUB_API_KEY", true),
        ("ALPACA_API_KEY", true),
        ("ALPACA_SECRET_KEY", true),
        ("ANTHROPIC_API_KEY", false),
        ("SOL_PRIVATE_KEY", false),
    ];

    let mut all_ok = true;
    for (var, required) in &checks {
        match std::env::var(var) {
            Ok(v) if !v.trim().is_empty() => {
                let display_val = if v.len() > 6 {
                    format!("{}...{}", &v[..3], &v[v.len() - 3..])
                } else {
                    "***".to_string()
                };
                println!("| [OK] {:<18} = {:<12}|", var, display_val);
            }
            _ if *required => {
                println!("| [!!] {:<18} -- MISSING       |", var);
                all_ok = false;
            }
            _ => {
                println!("| [--] {:<18} -- not set       |", var);
            }
        }
    }

    println!("├─────────────────────────────────────┤");

    // Also try to formally parse it to catch serialization errors
    match AppConfig::load() {
        Ok(config) => {
            println!("| [OK] AppConfig Parser: VALID          |");
            println!(
                "| [--] Endpoint: {:<22} |",
                if config.alpaca_base_url.contains("paper") {
                    "Paper (Safe)"
                } else {
                    "LIVE MARKET!"
                }
            );
            if config.use_mock == "1" {
                println!("| [!!] USE_MOCK=1 (Bypassing API reqs)  |");
            }
        }
        Err(e) => {
            println!("| [!!] AppConfig Parser: INVALID        |");
            println!(
                "│   Reason: {:<25} │",
                e.to_string().chars().take(25).collect::<String>()
            );
            all_ok = false;
        }
    }

    println!("└─────────────────────────────────────┘");

    if all_ok {
        println!("\n[OK] Configuration valid. You are ready to run the daemon.");
        std::process::exit(0);
    } else {
        println!("\n[!!] Missing required keys. See docs/SETUP.md for instructions.");
        std::process::exit(1);
    }
}
