use common::Action;
use solana_sdk::signature::Signature;
use tracing::info;

pub struct DryRunExecutor;

impl DryRunExecutor {
    pub fn execute_mock(&self, action: &Action) -> Signature {
        info!("[DRY RUN] Executing mock action: {:?}", action);
        // Simulate a successful execution by returning a dummy signature
        Signature::new_unique()
    }
}
