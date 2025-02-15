mod crawler;
mod initializer;
mod models;

use crate::crawler::{append_geolocation_data, crawl_network};
use crate::initializer::{Initializer, ROUTER};
use crate::models::NodeData;
use axum::http::{HeaderName, Method};
use axum::{extract::State, routing::get, Json, Router as AxumRouter};
use chrono::{DateTime, Utc};
use clap::{Parser, ValueEnum};
use serde::Serialize;
use spectre_p2p_lib::pb::spectred_message::Payload;
use spectre_p2p_lib::pb::{RequestAddressesMessage, SpectredMessage};
use spectre_p2p_lib::{make_message, Hub, Router as SpectreRouter};
use spectre_utils::hex::ToHex;
use spectre_utils::networking::IpAddress;
use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Receiver;
use tokio::sync::Mutex;
use tokio::time::sleep;
use tower_http::cors::{AllowOrigin, CorsLayer};

#[derive(ValueEnum, Clone, Debug, PartialEq)]
enum RequestType {
    Version,
    Addresses,
    Crawl,
}

#[derive(Parser, Clone, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(value_enum, help = "Request type (version, addresses, crawl)")]
    pub request: RequestType,

    #[clap(
        short = 's',
        long,
        default_value = "localhost:18111",
        help = "The ip:port of a spectred instance"
    )]
    pub url: String,

    #[clap(
        short,
        long,
        default_value = "mainnet",
        help = "The network type and suffix, e.g. 'testnet-11'"
    )]
    pub network: String,

    #[clap(
        short = 'o',
        long,
        default_value = "nodes.json",
        help = "Output JSON file (for crawl mode)"
    )]
    pub output: String,

    #[clap(
        long,
        help = "Port to serve the JSON API on (for crawl mode)",
        default_value = "3000"
    )]
    pub api_port: u16,
}

#[derive(Eq, PartialEq, Hash, Serialize)]
struct Version {
    pub protocol_version: u32,
    pub network: String,
    pub services: u64,
    pub timestamp: Option<DateTime<Utc>>,
    pub address: Option<NetAddress>,
    pub id: String,
    pub user_agent: String,
    pub disable_relay_tx: bool,
    pub subnetwork_id: Option<String>,
}

#[derive(Eq, PartialEq, Hash, Serialize)]
struct NetAddress {
    ip: IpAddr,
    port: u16,
}

struct AppState {
    nodes: Arc<Mutex<Vec<NodeData>>>,
}

async fn get_nodes(State(state): State<Arc<AppState>>) -> Json<Vec<NodeData>> {
    let nodes = state.nodes.lock().await;
    Json(nodes.clone())
}

/// update every 24 hours
async fn update_nodes_periodically(state: Arc<AppState>, cli_args: Arc<Cli>) {
    loop {
        println!("Updating node data...");

        let new_nodes = crawl_network(cli_args.clone()).await;
        let geolocated_nodes = append_geolocation_data(new_nodes, cli_args.clone()).await;

        {
            let mut nodes = state.nodes.lock().await;
            *nodes = geolocated_nodes;
        }

        println!("Node data updated. Waiting 24 hours for the next update...");
        sleep(Duration::from_secs(24 * 60 * 60)).await; // Wait
    }
}

async fn start_server(state: Arc<AppState>, port: u16) {
    // configure CORS
    let cors = CorsLayer::new()
        .allow_methods(vec![Method::GET])
        .allow_origin(AllowOrigin::any())
        .allow_headers(vec![
            HeaderName::from_static("content-type"),
            HeaderName::from_static("access-control-allow-origin"),
        ])
        .expose_headers(vec![HeaderName::from_static("content-type")]);

    let app = AxumRouter::new()
        .route("/nodes", get(get_nodes))
        .layer(cors)
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("Server running at http://{}", addr);

    let listener = TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

#[tokio::main]
async fn main() {
    let cli_args = Arc::new(Cli::parse());

    if cli_args.request == RequestType::Crawl {
        println!("Starting network crawl...");

        let initial_nodes = Vec::new(); // start with an empty list
        let state = Arc::new(AppState {
            nodes: Arc::new(Mutex::new(initial_nodes)),
        });

        // background task
        let state_clone = Arc::clone(&state);
        let cli_args_clone = Arc::clone(&cli_args);
        tokio::spawn(async move {
            update_nodes_periodically(state_clone, cli_args_clone).await;
        });

        // HTTP server
        start_server(state, cli_args.api_port).await;

        println!("Crawl complete. Data saved to {}", cli_args.output);
        return;
    }

    let (sender, mut receiver) = mpsc::channel(10);
    let initializer = Arc::new(Initializer::new(cli_args.clone(), sender));
    let adaptor =
        spectre_p2p_lib::Adaptor::client_only(Hub::new(), initializer, Default::default());

    if let Err(e) = adaptor
        .connect_peer_with_retries(cli_args.url.clone(), 3, Duration::from_secs(1))
        .await
    {
        panic!("Failed to connect to {}: {:?}", cli_args.url, e);
    }

    let router = ROUTER.read().unwrap().clone().unwrap();

    match cli_args.request {
        RequestType::Version => req_version(&mut receiver).await,
        RequestType::Addresses => req_addresses(&mut receiver, router).await,
        RequestType::Crawl => {}
    }
    adaptor.terminate_all_peers().await;
}

async fn req_version(receiver: &mut Receiver<SpectredMessage>) {
    loop {
        if let Some(msg) = receiver.recv().await {
            if let Some(Payload::Version(version_msg)) = msg.payload {
                let version = Version {
                    protocol_version: version_msg.protocol_version,
                    network: version_msg.network,
                    services: version_msg.services,
                    timestamp: DateTime::from_timestamp_millis(version_msg.timestamp),
                    address: version_msg.address.and_then(|a| a.try_into().ok()).map(
                        |(ip, port)| NetAddress {
                            ip: ip.to_canonical(),
                            port,
                        },
                    ),
                    id: version_msg.id.to_hex(),
                    user_agent: version_msg.user_agent,
                    disable_relay_tx: version_msg.disable_relay_tx,
                    subnetwork_id: version_msg.subnetwork_id.map(|s| s.bytes.to_hex()),
                };
                let json = serde_json::to_string(&version).unwrap();
                println!("{}", json);
                break;
            }
        }
    }
}

async fn req_addresses(receiver: &mut Receiver<SpectredMessage>, router: Arc<SpectreRouter>) {
    let _ = router
        .enqueue(make_message!(
            Payload::RequestAddresses,
            RequestAddressesMessage {
                include_all_subnetworks: false,
                subnetwork_id: None
            }
        ))
        .await;

    loop {
        if let Some(msg) = receiver.recv().await {
            if let Some(Payload::Addresses(addresses_msg)) = msg.payload {
                let mut addresses = HashSet::new();
                for address in addresses_msg.address_list {
                    if let Ok(result) = address.try_into() {
                        let (ip, port): (IpAddress, u16) = result;
                        addresses.insert(NetAddress {
                            ip: ip.to_canonical(),
                            port,
                        });
                    }
                }
                let json = serde_json::to_string(&addresses).unwrap();
                println!("{}", json);
                break;
            }
        }
    }
}
