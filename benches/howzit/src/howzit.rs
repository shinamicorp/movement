use std::ops::RangeInclusive;
use std::path::PathBuf;

use aptos_types::transaction::TransactionPayload;
use aptos_framework::BuildOptions;
use aptos_sdk::{
    move_types::{
        identifier::Identifier, language_storage::ModuleId
        
    }, rest_client::{Client, FaucetClient}, transaction_builder::TransactionBuilder, types::{chain_id::ChainId, transaction::EntryFunction, LocalAccount}
};
use anyhow::Context;
use std::time::{SystemTime, UNIX_EPOCH};
use crate::build_and_publish_package;
use tokio::sync::RwLock;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum Probe {
    Probe1,
    Probe2,
    Probe3
}

impl Probe {

    // associate constant for the probe range
    const PROBE_RANGE: RangeInclusive<u32> = 0..=3;

    /// Generates a pseudorandom probe such that lower values are exponentially more likely
    pub fn generate_exponential<R>(rng : &mut R) -> Self
        where R : rand::Rng
    {

        // generate a random number between 0 and 2^PROBE_RANGE.end
        let max = 2u32.pow(*Self::PROBE_RANGE.end());
        let random = rng.gen_range(0, max);

        // subtract the random number from 2^PROBE_RANGE.end to invert the distribution
        let inverse = 2u32.pow(*Self::PROBE_RANGE.end()) - random;

        // take the log base 2 of the result to get the probe
        // we'll use the leading zeros trick here
        let probe = u32::MAX - (inverse.leading_zeros() as u32);
        
        probe.into()

    }

}

impl From<u32> for Probe {
    fn from(value: u32) -> Self {
        let value = (
            value % Self::PROBE_RANGE.end()
        ) + Self::PROBE_RANGE.start();
        match value {
            1 => Probe::Probe1,
            2 => Probe::Probe2,
            3 => Probe::Probe3,
            _ => Probe::Probe1,
        }
    }
}

impl TryInto<Identifier> for Probe {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<Identifier, Self::Error> {
        match self {
            Probe::Probe1 => Ok(Identifier::new("probe_1")?),
            Probe::Probe2 => Ok(Identifier::new("probe_2")?),
            Probe::Probe3 => Ok(Identifier::new("probe_3")?),
        }
    }
}
#[derive(Clone)]
pub struct Howzit {
    pub howzit_package_path : PathBuf,
    pub wallet : Arc<RwLock<LocalAccount>>,
    pub rest_client : Client,
    pub faucet_client : Arc<RwLock<FaucetClient>>
}

impl Howzit {

    /// Generates a new Howzit instance with a random wallet
    pub fn generate(
        howzit_package_path: PathBuf,
        rest_client: Client,
        faucet_client: FaucetClient,
    ) -> Self {
        let wallet = LocalAccount::generate(&mut rand::rngs::OsRng);
        Howzit { 
            howzit_package_path, 
            wallet: Arc::new(RwLock::new(wallet)),
            rest_client,
            faucet_client: Arc::new(RwLock::new(faucet_client)),
        }
    }

    /// Builds and publishes the howzit package
    pub async fn build_and_publish(&self) -> Result<(), anyhow::Error> {

        let mut wallet = self.wallet.write().await;
        let mut faucet_client = self.faucet_client.write().await;

        // need to set the howzit address
        let mut build_options = BuildOptions::default();
        build_options.named_addresses.insert(
            "howzit".to_string(),
            wallet.address()
        );

        build_and_publish_package(
            &mut *wallet,
            self.rest_client.clone(),
            &*faucet_client,
            self.howzit_package_path.clone(),
            build_options
        ).await

    }

    /// Calls a generated probe function
    pub async fn call_probe(&self, alice : &mut LocalAccount) -> Result<(), anyhow::Error> {

        let chain_id = self.rest_client.get_index().await.context(
            "failed to get chain ID"
        )?.inner().chain_id;
        let wallet = self.wallet.read().await;
        let probe = Probe::generate_exponential(&mut rand::rngs::OsRng);

        let transaction_builder = TransactionBuilder::new(
            TransactionPayload::EntryFunction(EntryFunction::new(
                ModuleId::new(
                    wallet.address(), 
                    Identifier::new("howzit")?
                ),
                probe.clone().try_into()?,
                vec![],
                vec![],
            )),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)?
                .as_secs() + 60,
            ChainId::new(chain_id),
        )
        .sender(alice.address())
        .sequence_number(alice.sequence_number());
        let signed_txn = alice.sign_with_transaction_builder(transaction_builder);
    
        let txn_hash = self.rest_client
            .submit(&signed_txn)
            .await
            .context(
                format!("failed to submit transaction fo probe {:?}", probe)
            )?
            .into_inner();
        self.rest_client.wait_for_transaction(&txn_hash).await.context(
            format!("failed to wait for transaction for probe {:?}", probe)
        )?;

        Ok(())
    
    }

}