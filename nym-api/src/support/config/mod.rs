// Copyright 2021-2023 - Nym Technologies SA <contact@nymtech.net>
// SPDX-License-Identifier: GPL-3.0-only

use crate::support::config::persistence::{
    CoconutSignerPaths, NetworkMonitorPaths, NodeStatusAPIPaths,
};
use crate::support::config::template::CONFIG_TEMPLATE;
use nym_config::defaults::{mainnet, NymNetworkDetails};
use nym_config::{
    must_get_home, read_config_from_toml_file, save_formatted_config_to_file, NymConfigTemplate,
    DEFAULT_CONFIG_DIR, DEFAULT_CONFIG_FILENAME, DEFAULT_DATA_DIR, DEFAULT_NYM_APIS_DIR, NYM_DIR,
};
use nym_validator_client::nyxd;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;
use url::Url;
use zeroize::{Zeroize, ZeroizeOnDrop};

pub(crate) mod helpers;
pub(crate) mod old_config_v1_1_21;
pub(crate) mod old_config_v1_1_27;
mod persistence;
mod template;

pub const DEFAULT_LOCAL_VALIDATOR: &str = "http://localhost:26657";
pub const DEFAULT_NYM_API_PORT: u16 = 8080;

pub const DEFAULT_DKG_CONTRACT_POLLING_RATE: Duration = Duration::from_secs(10);

const DEFAULT_GATEWAY_SENDING_RATE: usize = 200;
const DEFAULT_MAX_CONCURRENT_GATEWAY_CLIENTS: usize = 50;
const DEFAULT_PACKET_DELIVERY_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_MONITOR_RUN_INTERVAL: Duration = Duration::from_secs(15 * 60);
const DEFAULT_GATEWAY_PING_INTERVAL: Duration = Duration::from_secs(60);
// Set this to a high value for now, so that we don't risk sporadic timeouts that might cause
// bought bandwidth tokens to not have time to be spent; Once we remove the gateway from the
// bandwidth bridging protocol, we can come back to a smaller timeout value
const DEFAULT_GATEWAY_RESPONSE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
// This timeout value should be big enough to accommodate an initial bandwidth acquirement
const DEFAULT_GATEWAY_CONNECTION_TIMEOUT: Duration = Duration::from_secs(2 * 60);

const DEFAULT_TEST_ROUTES: usize = 3;
const DEFAULT_MINIMUM_TEST_ROUTES: usize = 1;
const DEFAULT_ROUTE_TEST_PACKETS: usize = 1000;
const DEFAULT_PER_NODE_TEST_PACKETS: usize = 3;

const DEFAULT_TOPOLOGY_CACHE_INTERVAL: Duration = Duration::from_secs(30);
const DEFAULT_NODE_STATUS_CACHE_INTERVAL: Duration = Duration::from_secs(120);
const DEFAULT_CIRCULATING_SUPPLY_CACHE_INTERVAL: Duration = Duration::from_secs(3600);

pub(crate) const DEFAULT_NODE_DESCRIBE_CACHE_INTERVAL: Duration = Duration::from_secs(4500);
pub(crate) const DEFAULT_NODE_DESCRIBE_BATCH_SIZE: usize = 50;

const DEFAULT_MONITOR_THRESHOLD: u8 = 60;
const DEFAULT_MIN_MIXNODE_RELIABILITY: u8 = 50;
const DEFAULT_MIN_GATEWAY_RELIABILITY: u8 = 20;

/// Derive default path to nym-api's config directory.
/// It should get resolved to `$HOME/.nym/nym-api/<id>/config`
pub fn default_config_directory<P: AsRef<Path>>(id: P) -> PathBuf {
    must_get_home()
        .join(NYM_DIR)
        .join(DEFAULT_NYM_APIS_DIR)
        .join(id)
        .join(DEFAULT_CONFIG_DIR)
}

/// Derive default path to nym-api's config file.
/// It should get resolved to `$HOME/.nym/nym-api/<id>/config/config.toml`
pub fn default_config_filepath<P: AsRef<Path>>(id: P) -> PathBuf {
    default_config_directory(id).join(DEFAULT_CONFIG_FILENAME)
}

/// Derive default path to nym-api's data directory where files, such as keys, are stored.
/// It should get resolved to `$HOME/.nym/nym-api/<id>/data`
pub fn default_data_directory<P: AsRef<Path>>(id: P) -> PathBuf {
    must_get_home()
        .join(NYM_DIR)
        .join(DEFAULT_NYM_APIS_DIR)
        .join(id)
        .join(DEFAULT_DATA_DIR)
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Config {
    pub base: Base,

    // TODO: perhaps introduce separate 'path finder' field for all the paths and directories like we have with other configs
    pub network_monitor: NetworkMonitor,

    pub node_status_api: NodeStatusAPI,

    pub topology_cacher: TopologyCacher,

    pub circulating_supply_cacher: CirculatingSupplyCacher,

    pub rewarding: Rewarding,

    pub coconut_signer: CoconutSigner,

    pub ephemera: Ephemera,
}

impl<'a> From<&'a Config> for NymNetworkDetails {
    fn from(value: &'a Config) -> Self {
        // we get the current environmental details and then overwrite whatever is appropriate with
        // the values from the config
        NymNetworkDetails::new_from_env()
            .with_mixnet_contract(Some(value.get_mixnet_contract_address().as_ref()))
            .with_vesting_contract(Some(value.get_vesting_contract_address().as_ref()))
    }
}

impl NymConfigTemplate for Config {
    fn template(&self) -> &'static str {
        CONFIG_TEMPLATE
    }
}

impl Config {
    pub fn new<S: AsRef<str>>(id: S) -> Self {
        Config {
            base: Base::new_default(id.as_ref()),
            network_monitor: NetworkMonitor::new_default(id.as_ref()),
            node_status_api: NodeStatusAPI::new_default(id.as_ref()),
            topology_cacher: Default::default(),
            circulating_supply_cacher: Default::default(),
            rewarding: Default::default(),
            coconut_signer: CoconutSigner::new_default(id.as_ref()),
            ephemera: Ephemera::new_default(id.as_ref()),
        }
    }

    pub fn read_from_toml_file<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        read_config_from_toml_file(path)
    }

    pub fn read_from_default_path<P: AsRef<Path>>(id: P) -> io::Result<Self> {
        Self::read_from_toml_file(default_config_filepath(id))
    }

    pub fn default_location(&self) -> PathBuf {
        default_config_filepath(&self.base.id)
    }

    pub fn save_to_default_location(&self) -> io::Result<()> {
        let config_save_location: PathBuf = self.default_location();
        save_formatted_config_to_file(self, config_save_location)
    }

    pub fn get_network_details(&self) -> NymNetworkDetails {
        self.into()
    }

    pub fn with_network_monitor_enabled(mut self, enabled: bool) -> Self {
        self.network_monitor.enabled = enabled;
        self
    }

    pub fn with_disabled_credentials_mode(mut self, disabled_credentials_mode: bool) -> Self {
        self.network_monitor.debug.disabled_credentials_mode = disabled_credentials_mode;
        self
    }

    pub fn with_rewarding_enabled(mut self, enabled: bool) -> Self {
        self.rewarding.enabled = enabled;
        self
    }

    pub fn with_coconut_signer_enabled(mut self, enabled: bool) -> Self {
        self.coconut_signer.enabled = enabled;
        self
    }

    pub fn with_ephemera_enabled(mut self, enabled: bool) -> Self {
        self.ephemera.enabled = enabled;
        self
    }

    pub fn with_custom_nyxd_validator(mut self, validator: Url) -> Self {
        self.base.local_validator = validator;
        self
    }

    pub fn with_announce_address(mut self, announce_address: Url) -> Self {
        self.coconut_signer.announce_address = announce_address;
        self
    }

    pub fn with_custom_mixnet_contract(mut self, mixnet_contract: nyxd::AccountId) -> Self {
        self.base.mixnet_contract_address = mixnet_contract;
        self
    }

    pub fn with_custom_vesting_contract(mut self, vesting_contract: nyxd::AccountId) -> Self {
        self.base.vesting_contract_address = vesting_contract;
        self
    }

    pub fn with_mnemonic(mut self, mnemonic: bip39::Mnemonic) -> Self {
        self.base.mnemonic = mnemonic;
        self
    }

    pub fn with_minimum_interval_monitor_threshold(mut self, threshold: u8) -> Self {
        self.rewarding.debug.minimum_interval_monitor_threshold = threshold;
        self
    }

    pub fn with_min_mixnode_reliability(mut self, min_mixnode_reliability: u8) -> Self {
        self.network_monitor.debug.min_mixnode_reliability = min_mixnode_reliability;
        self
    }

    pub fn with_min_gateway_reliability(mut self, min_gateway_reliability: u8) -> Self {
        self.network_monitor.debug.min_gateway_reliability = min_gateway_reliability;
        self
    }

    pub fn with_ephemera_ip(mut self, ip: String) -> Self {
        self.ephemera.args.cmd.ephemera_ip = Some(ip);
        self
    }

    pub fn with_ephemera_protocol_port(mut self, port: u16) -> Self {
        self.ephemera.args.cmd.ephemera_protocol_port = Some(port);
        self
    }

    pub fn with_ephemera_websocket_port(mut self, port: u16) -> Self {
        self.ephemera.args.cmd.ephemera_websocket_port = Some(port);
        self
    }

    pub fn with_ephemera_http_api_port(mut self, port: u16) -> Self {
        self.ephemera.args.cmd.ephemera_http_api_port = Some(port);
        self
    }

    pub fn get_id(&self) -> String {
        self.base.id.clone()
    }

    pub fn get_nyxd_url(&self) -> Url {
        self.base.local_validator.clone()
    }

    pub fn get_mixnet_contract_address(&self) -> nyxd::AccountId {
        self.base.mixnet_contract_address.clone()
    }

    pub fn get_vesting_contract_address(&self) -> nyxd::AccountId {
        self.base.vesting_contract_address.clone()
    }

    pub fn get_mnemonic(&self) -> bip39::Mnemonic {
        self.base.mnemonic.clone()
    }

    pub fn get_ephemera_args(&self) -> &crate::ephemera::Args {
        &self.ephemera.args
    }

    pub fn get_ephemera_config_path(&self) -> PathBuf {
        self.ephemera.args.ephemera_config.clone()
    }
}

// we only really care about the mnemonic being zeroized
#[derive(Debug, Deserialize, PartialEq, Eq, Serialize, Zeroize, ZeroizeOnDrop)]
pub struct Base {
    /// ID specifies the human readable ID of this particular nym-api.
    id: String,

    #[zeroize(skip)]
    local_validator: Url,

    /// Address of the validator contract managing the network
    #[zeroize(skip)]
    mixnet_contract_address: nyxd::AccountId,

    /// Address of the vesting contract holding locked tokens
    #[zeroize(skip)]
    vesting_contract_address: nyxd::AccountId,

    /// Mnemonic used for rewarding and/or multisig operations
    // TODO: similarly to the note in gateway, this should get moved to a separate file
    mnemonic: bip39::Mnemonic,
}

impl Base {
    pub fn new_default<S: Into<String>>(id: S) -> Self {
        let default_validator: Url = DEFAULT_LOCAL_VALIDATOR
            .parse()
            .expect("default local validator is malformed!");

        Base {
            id: id.into(),
            local_validator: default_validator,
            mixnet_contract_address: mainnet::MIXNET_CONTRACT_ADDRESS.parse().unwrap(),
            vesting_contract_address: mainnet::VESTING_CONTRACT_ADDRESS.parse().unwrap(),
            // this this doesn't make any sense since you really have a mnemonic beforehand...
            mnemonic: bip39::Mnemonic::generate(24).unwrap(),
        }
    }
}

// this got separated into 2 structs so that we could have a sane `default` implementation for the latter
#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct NetworkMonitor {
    /// Specifies whether network monitoring service is enabled in this process.
    pub enabled: bool,

    pub storage_paths: NetworkMonitorPaths,

    #[serde(default)]
    pub debug: NetworkMonitorDebug,
}

impl NetworkMonitor {
    pub fn new_default<P: AsRef<Path>>(id: P) -> Self {
        NetworkMonitor {
            enabled: false,
            storage_paths: NetworkMonitorPaths::new_default(id),
            debug: Default::default(),
        }
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default)]
pub struct NetworkMonitorDebug {
    //  Mixnodes and gateways with reliability lower the this get blacklisted by network monitor, get no traffic and cannot be selected into a rewarded set.
    pub min_mixnode_reliability: u8, // defaults to 50
    pub min_gateway_reliability: u8, // defaults to 20

    /// Indicates whether this validator api is running in a disabled credentials mode, thus attempting
    /// to claim bandwidth without presenting bandwidth credentials.
    pub disabled_credentials_mode: bool,

    /// Specifies the interval at which the network monitor sends the test packets.
    #[serde(with = "humantime_serde")]
    pub run_interval: Duration,

    /// Specifies interval at which we should be sending ping packets to all active gateways
    /// in order to keep the websocket connections alive.
    #[serde(with = "humantime_serde")]
    pub gateway_ping_interval: Duration,

    /// Specifies maximum rate (in packets per second) of test packets being sent to gateway
    pub gateway_sending_rate: usize,

    /// Maximum number of gateway clients the network monitor will try to talk to concurrently.
    /// 0 = no limit
    pub max_concurrent_gateway_clients: usize,

    /// Maximum allowed time for receiving gateway response.
    #[serde(with = "humantime_serde")]
    pub gateway_response_timeout: Duration,

    /// Maximum allowed time for the gateway connection to get established.
    #[serde(with = "humantime_serde")]
    pub gateway_connection_timeout: Duration,

    /// Specifies the duration the monitor is going to wait after sending all measurement
    /// packets before declaring nodes unreachable.
    #[serde(with = "humantime_serde")]
    pub packet_delivery_timeout: Duration,

    /// Desired number of test routes to be constructed (and working) during a monitor test run.
    pub test_routes: usize,

    /// The minimum number of test routes that need to be constructed (and working) in order for
    /// a monitor test run to be valid.
    pub minimum_test_routes: usize,

    /// Number of test packets sent via each pseudorandom route to verify whether they work correctly,
    /// before using them for testing the rest of the network.
    pub route_test_packets: usize,

    /// Number of test packets sent to each node during regular monitor test run.
    pub per_node_test_packets: usize,
}

impl Default for NetworkMonitorDebug {
    fn default() -> Self {
        NetworkMonitorDebug {
            min_mixnode_reliability: DEFAULT_MIN_MIXNODE_RELIABILITY,
            min_gateway_reliability: DEFAULT_MIN_GATEWAY_RELIABILITY,
            disabled_credentials_mode: true,
            run_interval: DEFAULT_MONITOR_RUN_INTERVAL,
            gateway_ping_interval: DEFAULT_GATEWAY_PING_INTERVAL,
            gateway_sending_rate: DEFAULT_GATEWAY_SENDING_RATE,
            max_concurrent_gateway_clients: DEFAULT_MAX_CONCURRENT_GATEWAY_CLIENTS,
            gateway_response_timeout: DEFAULT_GATEWAY_RESPONSE_TIMEOUT,
            gateway_connection_timeout: DEFAULT_GATEWAY_CONNECTION_TIMEOUT,
            packet_delivery_timeout: DEFAULT_PACKET_DELIVERY_TIMEOUT,
            test_routes: DEFAULT_TEST_ROUTES,
            minimum_test_routes: DEFAULT_MINIMUM_TEST_ROUTES,
            route_test_packets: DEFAULT_ROUTE_TEST_PACKETS,
            per_node_test_packets: DEFAULT_PER_NODE_TEST_PACKETS,
        }
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct NodeStatusAPI {
    // pub enabled: bool,
    pub storage_paths: NodeStatusAPIPaths,

    #[serde(default)]
    pub debug: NodeStatusAPIDebug,
}

impl NodeStatusAPI {
    pub fn new_default<P: AsRef<Path>>(id: P) -> Self {
        NodeStatusAPI {
            storage_paths: NodeStatusAPIPaths::new_default(id),
            debug: Default::default(),
        }
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default)]
pub struct NodeStatusAPIDebug {
    // TODO: allow for this...
    // port: u16,
    #[serde(with = "humantime_serde")]
    pub caching_interval: Duration,
}

impl Default for NodeStatusAPIDebug {
    fn default() -> Self {
        NodeStatusAPIDebug {
            caching_interval: DEFAULT_NODE_STATUS_CACHE_INTERVAL,
        }
    }
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default)]
pub struct TopologyCacher {
    // pub enabled: bool,

    // pub paths: TopologyCacherPathfinder,
    #[serde(default)]
    pub debug: TopologyCacherDebug,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default)]
pub struct TopologyCacherDebug {
    #[serde(with = "humantime_serde")]
    pub caching_interval: Duration,

    #[serde(with = "humantime_serde")]
    pub node_describe_caching_interval: Duration,

    pub node_describe_batch_size: usize,
}

impl Default for TopologyCacherDebug {
    fn default() -> Self {
        TopologyCacherDebug {
            caching_interval: DEFAULT_TOPOLOGY_CACHE_INTERVAL,
            node_describe_caching_interval: DEFAULT_NODE_DESCRIBE_CACHE_INTERVAL,
            node_describe_batch_size: DEFAULT_NODE_DESCRIBE_BATCH_SIZE,
        }
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default)]
pub struct CirculatingSupplyCacher {
    pub enabled: bool,

    // pub paths: CirculatingSupplyCacherPathfinder,
    #[serde(default)]
    pub debug: CirculatingSupplyCacherDebug,
}

impl Default for CirculatingSupplyCacher {
    fn default() -> Self {
        CirculatingSupplyCacher {
            enabled: true,
            debug: CirculatingSupplyCacherDebug::default(),
        }
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default)]
pub struct CirculatingSupplyCacherDebug {
    #[serde(with = "humantime_serde")]
    pub caching_interval: Duration,
}

impl Default for CirculatingSupplyCacherDebug {
    fn default() -> Self {
        CirculatingSupplyCacherDebug {
            caching_interval: DEFAULT_CIRCULATING_SUPPLY_CACHE_INTERVAL,
        }
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default)]
pub struct Rewarding {
    /// Specifies whether rewarding service is enabled in this process.
    pub enabled: bool,

    // this should really be a thing too...
    // pub paths: RewardingPathfinder,
    #[serde(default)]
    pub debug: RewardingDebug,
}

#[allow(clippy::derivable_impls)]
impl Default for Rewarding {
    fn default() -> Self {
        Rewarding {
            enabled: false,
            debug: Default::default(),
        }
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default)]
pub struct RewardingDebug {
    /// Specifies the minimum percentage of monitor test run data present in order to
    /// distribute rewards for given interval.
    /// Note, only values in range 0-100 are valid
    pub minimum_interval_monitor_threshold: u8,
}

impl Default for RewardingDebug {
    fn default() -> Self {
        RewardingDebug {
            minimum_interval_monitor_threshold: DEFAULT_MONITOR_THRESHOLD,
        }
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct CoconutSigner {
    /// Specifies whether rewarding service is enabled in this process.
    pub enabled: bool,

    pub announce_address: Url,

    pub storage_paths: CoconutSignerPaths,

    #[serde(default)]
    pub debug: CoconutSignerDebug,
}

impl CoconutSigner {
    pub fn new_default<P: AsRef<Path>>(id: P) -> Self {
        let default_validator: Url = DEFAULT_LOCAL_VALIDATOR
            .parse()
            .expect("default local validator is malformed!");
        let mut default_announce_address = default_validator;
        default_announce_address
            .set_port(Some(DEFAULT_NYM_API_PORT))
            .expect("default local validator is malformed!");

        CoconutSigner {
            enabled: false,
            announce_address: default_announce_address,
            storage_paths: CoconutSignerPaths::new_default(id),
            debug: Default::default(),
        }
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default)]
pub struct CoconutSignerDebug {
    /// Duration of the interval for polling the dkg contract.
    #[serde(with = "humantime_serde")]
    pub dkg_contract_polling_rate: Duration,
}

impl Default for CoconutSignerDebug {
    fn default() -> Self {
        CoconutSignerDebug {
            dkg_contract_polling_rate: DEFAULT_DKG_CONTRACT_POLLING_RATE,
        }
    }
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default)]
pub struct Ephemera {
    pub enabled: bool,
    args: crate::ephemera::Args,
}

impl Ephemera {
    fn new_default(id: &str) -> Self {
        Ephemera {
            enabled: false,
            args: crate::ephemera::Args {
                ephemera_config: ephemera::configuration::Configuration::ephemera_config_file_home(
                    Some(id),
                )
                .unwrap(),
                ..Default::default()
            },
        }
    }
}
