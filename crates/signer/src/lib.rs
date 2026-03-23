#![forbid(unsafe_code)]
use solana_sdk::signature::{Keypair, Signer};
use anyhow::Result;

pub struct LocalSigner {
    keypair: Keypair,
}

impl LocalSigner {
    pub fn new(keypair: Keypair) -> Self {
        Self { keypair }
    }

    pub fn from_base58(secret: &str) -> Result<Self> {
        let keypair = Keypair::from_base58_string(secret);
        Ok(Self { keypair })
    }

    pub fn from_file(path: &str) -> Result<Self> {
        let keypair = solana_sdk::signature::read_keypair_file(path)
            .map_err(|e| anyhow::anyhow!("Failed to read keypair file: {:?}", e))?;
        Ok(Self { keypair })
    }

    pub fn pubkey(&self) -> solana_sdk::pubkey::Pubkey {
        self.keypair.pubkey()
    }

    pub fn sign_transaction(&self, tx: &mut solana_sdk::transaction::Transaction) {
        let signers = vec![&self.keypair];
        tx.partial_sign(&signers, tx.message.recent_blockhash);
    }
}
