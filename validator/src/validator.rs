use crate::network::tendermint;
use crypto::identity::{
    DummyMixIdentityKeyPair, DummyMixIdentityPrivateKey, DummyMixIdentityPublicKey,
    MixnetIdentityKeyPair, MixnetIdentityPrivateKey, MixnetIdentityPublicKey,
};
use healthcheck::HealthChecker;
use tokio::runtime::Runtime;

use serde_derive::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Config {
    #[serde(rename(deserialize = "healthcheck"))]
    pub health_check: healthcheck::config::HealthCheck,
}

// allow for a generic validator
pub struct Validator<IDPair, Priv, Pub>
where
    IDPair: MixnetIdentityKeyPair<Priv, Pub>,
    Priv: MixnetIdentityPrivateKey,
    Pub: MixnetIdentityPublicKey,
{
    tendermint_abci: tendermint::Abci,
    health_check: HealthChecker<IDPair, Priv, Pub>,
    #[allow(dead_code)]
    identity_keypair: IDPair,
}

// but for time being, since it's a dummy one, have it use dummy keys
impl Validator<DummyMixIdentityKeyPair, DummyMixIdentityPrivateKey, DummyMixIdentityPublicKey> {
    pub fn new(config: Config) -> Self {
        let dummy_keypair = DummyMixIdentityKeyPair::new();

        Validator {
            tendermint_abci: tendermint::Abci::new(),
            health_check: HealthChecker::new(config.health_check, dummy_keypair.clone()),
            identity_keypair: dummy_keypair,
        }
    }

    pub fn start(self) {
        let mut rt = Runtime::new().unwrap();

        let abci_future = self.tendermint_abci.run();
        let health_check_future = self.health_check.run();

        rt.block_on(abci_future);

        let health_check_res = rt.block_on(health_check_future);
        assert!(health_check_res.is_ok()); // panic if health checker failed
    }
}
