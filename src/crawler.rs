use crate::initializer::{Initializer, ROUTER};
use crate::models::{NodeData, NodeMetadata};
use crate::Cli;
use futures::future::join_all;
use serde::Serialize;
use serde_json::Value;
use spectre_p2p_lib::pb::spectred_message::Payload;
use spectre_p2p_lib::pb::RequestAddressesMessage;
use spectre_p2p_lib::{make_message, Hub};
use spectre_utils::hex::ToHex;
use spectre_utils::networking::IpAddress;
use std::collections::{HashSet, VecDeque};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{sleep, timeout};

#[derive(Eq, PartialEq, Hash, Serialize, Clone)]
pub struct NetAddress {
    ip: IpAddr,
    port: u16,
}

/// crawl from the node provided in cli_args.url
pub async fn crawl_network(cli_args: Arc<Cli>) -> Vec<NodeData> {
    let discovered_peers = Arc::new(Mutex::new(HashSet::new()));
    let results = Arc::new(Mutex::new(Vec::new()));
    let queue = Arc::new(Mutex::new(VecDeque::new()));

    println!("Starting crawl from: {}", cli_args.url);
    queue.lock().await.push_back(cli_args.url.clone());

    while !queue.lock().await.is_empty() {
        // drain current queue into a batch for concurrent processing
        let batch = {
            let mut queue_lock = queue.lock().await;
            queue_lock.drain(..).collect::<Vec<String>>()
        };

        println!("Batch size: {}", batch.len());
        let tasks = batch
            .into_iter()
            .map(|url| {
                let cli_args = cli_args.clone();
                let discovered_peers = Arc::clone(&discovered_peers);
                let results = Arc::clone(&results);
                let queue = Arc::clone(&queue);

                tokio::spawn(async move {
                    println!("Querying: {}", url);
                    match connect_and_query_peer(cli_args, &url).await {
                        Ok((url, peers, Some(metadata))) => {
                            println!("{}: Handshake OK, {} peers", url, peers.len());
                            results.lock().await.push(NodeData {
                                ip: url.clone(),
                                metadata,
                                loc: None,
                            });

                            let mut discovered_lock = discovered_peers.lock().await;
                            let mut queue_lock = queue.lock().await;
                            for peer in peers {
                                if !discovered_lock.contains(&peer) {
                                    discovered_lock.insert(peer.clone());
                                    let peer_addr = format!("{}:{}", peer.ip, peer.port);
                                    queue_lock.push_back(peer_addr.clone());
                                    println!("New peer: {}", peer_addr);
                                }
                            }
                        }
                        Ok((url, _, None)) => println!("{}: Handshake failed", url),
                        Err(_) => println!("{}: Query failed", url),
                    }
                })
            })
            .collect::<Vec<_>>();

        // If no tasks were spawned, there are no new nodes to query
        if tasks.is_empty() {
            println!("No new peers to query, exiting...");
            break;
        } else {
            println!("Awaiting {} queries", tasks.len());
        }

        // Await all tasks concurrently using join_all
        // This is the batch processing step: all node queries in the current batch run in parallel
        join_all(tasks).await;
        println!("Batch complete, checking queue");
    }

    println!("Finalizing results...");
    // serialize collected results to JSON
    let results_guard = results.lock().await;
    let nodes = (*results_guard).clone();
    drop(results_guard);

    add_geolocation(nodes, cli_args.clone()).await
}

/// Query a single node at the given URL and return its discovered addresses and handshake metadata
/// returns error if handshake fails
async fn connect_and_query_peer(
    cli_args: Arc<Cli>,
    url: &str,
) -> Result<(String, Vec<NetAddress>, Option<NodeMetadata>), ()> {
    println!("Connecting to node: {}", url);

    // channel to receive messages from peer
    let (sender, mut receiver) = mpsc::channel(10);
    let initializer = Arc::new(Initializer::new(cli_args.clone(), sender));
    let adaptor =
        spectre_p2p_lib::Adaptor::client_only(Hub::new(), initializer, Default::default());

    if adaptor.connect_peer(url.to_string()).await.is_err() {
        println!("Peer {}: connection failed", url);
        adaptor.terminate_all_peers().await;
        return Err(());
    }

    // Retrieve shared router from global state
    let router = {
        let router_guard = ROUTER.read().unwrap();
        router_guard.clone()
    };
    let router = if let Some(router) = router {
        router
    } else {
        println!("Peer {}: router not initialized, skipping", url);
        adaptor.terminate_all_peers().await;
        return Err(());
    };

    // Send a request to get peers list
    let _ = router
        .enqueue(make_message!(
            Payload::RequestAddresses,
            RequestAddressesMessage {
                include_all_subnetworks: false,
                subnetwork_id: None
            }
        ))
        .await;

    let mut addresses = Vec::new();
    let mut metadata: Option<NodeMetadata> = None;

    // Listen for responses with a timeout to avoid hanging indefinitely
    loop {
        match timeout(Duration::from_secs(3), receiver.recv()).await {
            Ok(Some(msg)) => match msg.payload {
                Some(Payload::Addresses(addresses_msg)) => {
                    for address in addresses_msg.address_list {
                        if let Ok(result) = address.try_into() {
                            let (ip, port): (IpAddress, u16) = result;
                            addresses.push(NetAddress {
                                ip: ip.to_canonical(),
                                port,
                            });
                        }
                    }
                    println!("Peer {}: received {} addresses", url, addresses.len());
                }
                Some(Payload::Version(version_msg)) => {
                    metadata = Some(NodeMetadata {
                        protocol_version: version_msg.protocol_version,
                        network: version_msg.network,
                        services: version_msg.services,
                        timestamp: chrono::Utc::now().to_rfc3339(),
                        id: version_msg.id.to_hex(),
                        user_agent: version_msg.user_agent,
                        disable_relay_tx: version_msg.disable_relay_tx,
                    });
                    println!("Peer {}: received metadata", url);
                }
                _ => {}
            },
            Ok(None) => break,
            Err(_) => {
                println!("Peer {}: timeout waiting for messages", url);
                break;
            }
        }
    }

    println!("Peer {}: disconnected", url);
    adaptor.terminate_all_peers().await;

    // only return nodes that completed handshake successfully
    if metadata.is_some() {
        Ok((url.to_string(), addresses, metadata))
    } else {
        Err(())
    }
}

pub async fn add_geolocation(data: Vec<NodeData>, _cli_args: Arc<Cli>) -> Vec<NodeData> {
    println!("Total to geolocate: {}", data.len());
    let mut geolocated_nodes = Vec::new();

    for node in data {
        let ip = node.ip.split(':').next().unwrap_or(&node.ip).to_string();
        let url = format!("http://ip-api.com/json/{}?fields=lat,lon,status", ip);
        let mut updated_node = node.clone();

        println!("Geolocating: {}", updated_node.ip);

        let mut retries = 3;
        let mut success = false;

        while retries > 0 {
            match reqwest::get(&url).await {
                Ok(response) => match response.json::<Value>().await {
                    Ok(json) if json["status"] == "success" => {
                        if let (Some(lat), Some(lon)) = (json["lat"].as_f64(), json["lon"].as_f64())
                        {
                            updated_node.loc = Some(format!("{},{}", lat, lon));
                            success = true;
                            break;
                        }
                    }
                    Ok(json) => {
                        eprintln!(
                            "Peer {}: geolocation failed, status: {}",
                            updated_node.ip, json["status"]
                        );
                    }
                    Err(e) => {
                        eprintln!("Peer {}: JSON parse error: {}", updated_node.ip, e);
                    }
                },
                Err(e) => {
                    eprintln!("Peer {}: HTTP error: {}", updated_node.ip, e);
                }
            }

            retries -= 1;
            if retries > 0 {
                println!(
                    "Peer {}: retrying in 1s ({} left)",
                    updated_node.ip, retries
                );
                sleep(Duration::from_secs(1)).await;
            }
        }

        if !success {
            eprintln!("Peer {}: geolocation failed", updated_node.ip);
        }

        geolocated_nodes.push(updated_node);

        // 1s pause to respect rate limits
        sleep(Duration::from_secs(1)).await;
    }

    geolocated_nodes
}
