// Copyright 2020-2023 - Nym Technologies SA <contact@nymtech.net>
// SPDX-License-Identifier: GPL-3.0-only

#[macro_use]
extern crate rocket;

use crate::epoch_operations::RewardedSetUpdater;
use crate::network::models::NetworkDetails;
use crate::node_describe_cache::DescribedNodes;
use crate::node_status_api::uptime_updater::HistoricalUptimeUpdater;
use crate::support::caching::cache::SharedCache;
use crate::support::cli::{self, Commands};
use crate::support::config::Config;
use crate::support::storage;
use crate::support::storage::NymApiStorage;
use ::ephemera::configuration::Configuration as EphemeraConfiguration;
use ::nym_config::defaults::setup_env;
use anyhow::Result;
use circulating_supply_api::cache::CirculatingSupplyCache;
use clap::Parser;
use coconut::dkg::controller::DkgController;
use node_status_api::NodeStatusCache;
use nym_bin_common::logging::setup_logging;
use nym_contract_cache::cache::NymContractCache;
use nym_sphinx::receiver::SphinxMessageReceiver;
use nym_task::TaskManager;
use rand::rngs::OsRng;
use std::error::Error;
use support::{http, nyxd};

mod circulating_supply_api;
mod coconut;
mod ephemera;
mod epoch_operations;
pub(crate) mod network;
mod network_monitor;
pub(crate) mod node_describe_cache;
pub(crate) mod node_status_api;
pub(crate) mod nym_contract_cache;
pub(crate) mod nym_nodes;
pub(crate) mod support;

struct ShutdownHandles {
    task_manager_handle: TaskManager,
    rocket_handle: rocket::Shutdown,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    println!("Starting nym api...");

    cfg_if::cfg_if! {if #[cfg(feature = "console-subscriber")] {
        // instrument tokio console subscriber needs RUSTFLAGS="--cfg tokio_unstable" at build time
        console_subscriber::init();
    }}

    setup_logging();
    let args = cli::Cli::parse();
    trace!("{:#?}", args);

    setup_env(args.config_env_file.as_ref());

    let command = args.command.unwrap_or(Commands::Run(Box::new(args.run)));

    match command {
        Commands::BuildInfo(m) => {
            cli::build_info::execute(m);
            Ok(())
        }
        Commands::Run(m) => cli::run::execute(*m).await,
    }
}

async fn start_nym_api_tasks(
    config: Config,
) -> Result<ShutdownHandles, Box<dyn Error + Send + Sync>> {
    let nyxd_client = nyxd::Client::new(&config);
    let connected_nyxd = config.get_nyxd_url();
    let nym_network_details = config.get_network_details();
    let network_details = NetworkDetails::new(connected_nyxd.to_string(), nym_network_details);

    let coconut_keypair = coconut::keypair::KeyPair::new();

    // let's build our rocket!
    let rocket = http::setup_rocket(
        &config,
        network_details,
        nyxd_client.clone(),
        coconut_keypair.clone(),
    )
    .await?;

    // setup shutdowns
    let shutdown = TaskManager::new(10);

    // Rocket handles shutdown on its own, but its shutdown handling should be incorporated
    // with that of the rest of the tasks. Currently its runtime is forcefully terminated once
    // nym-api exits.
    let rocket_shutdown_handle = rocket.shutdown();

    // get references to the managed state
    let nym_contract_cache_state = rocket.state::<NymContractCache>().unwrap();
    let node_status_cache_state = rocket.state::<NodeStatusCache>().unwrap();
    let circulating_supply_cache_state = rocket.state::<CirculatingSupplyCache>().unwrap();
    let maybe_storage = rocket.state::<NymApiStorage>();
    let described_nodes_state = rocket.state::<SharedCache<DescribedNodes>>().unwrap();

    // start note describe cache refresher
    // we should be doing the below, but can't due to our current startup structure
    // let refresher = node_describe_cache::new_refresher(&config.topology_cacher);
    // let cache = refresher.get_shared_cache();
    node_describe_cache::new_refresher_with_initial_value(
        &config.topology_cacher,
        nym_contract_cache_state.clone(),
        described_nodes_state.to_owned(),
    )
    .named("node-self-described-data-refresher")
    .start(shutdown.subscribe_named("node-self-described-data-refresher"));

    // start all the caches first
    let nym_contract_cache_listener = nym_contract_cache::start_refresher(
        &config.node_status_api,
        nym_contract_cache_state,
        nyxd_client.clone(),
        &shutdown,
    );

    node_status_api::start_cache_refresh(
        &config.node_status_api,
        nym_contract_cache_state,
        node_status_cache_state,
        maybe_storage,
        nym_contract_cache_listener,
        &shutdown,
    );
    circulating_supply_api::start_cache_refresh(
        &config.circulating_supply_cacher,
        nyxd_client.clone(),
        circulating_supply_cache_state,
        &shutdown,
    );

    // start dkg task
    if config.coconut_signer.enabled {
        DkgController::start(
            &config.coconut_signer,
            nyxd_client.clone(),
            coconut_keypair,
            OsRng,
            &shutdown,
        )
        .await?;
    }

    // and then only start the uptime updater (and the monitor itself, duh)
    // if the monitoring if it's enabled
    if config.network_monitor.enabled {
        let ephemera_config =
            match EphemeraConfiguration::try_load(config.get_ephemera_config_path()) {
                Ok(c) => c,
                Err(_) => {
                    config
                        .get_ephemera_args()
                        .cmd
                        .clone()
                        .execute(Some(&config.get_id()));
                    EphemeraConfiguration::try_load(config.get_ephemera_config_path())
                        .expect("Config file should be created now")
                }
            };
        let ephemera_reward_manager = if config.ephemera.enabled {
            Some(
                ephemera::application::NymApi::run(
                    config.get_ephemera_args().clone(),
                    ephemera_config,
                    nyxd_client.clone(),
                    &shutdown,
                )
                .await?,
            )
        } else {
            None
        };

        // if network monitor is enabled, the storage MUST BE available
        let storage = maybe_storage.unwrap();

        network_monitor::start::<SphinxMessageReceiver>(
            &config.network_monitor,
            nym_contract_cache_state,
            storage,
            nyxd_client.clone(),
            &shutdown,
        )
        .await;

        HistoricalUptimeUpdater::start(storage, &shutdown);

        // start 'rewarding' if its enabled
        if config.rewarding.enabled {
            epoch_operations::ensure_rewarding_permission(&nyxd_client).await?;
            RewardedSetUpdater::start(
                ephemera_reward_manager,
                nyxd_client,
                nym_contract_cache_state,
                storage,
                &shutdown,
            );
        }
    }

    // Launch the rocket, serve http endpoints and finish the startup
    tokio::spawn(rocket.launch());

    Ok(ShutdownHandles {
        task_manager_handle: shutdown,
        rocket_handle: rocket_shutdown_handle,
    })
}
