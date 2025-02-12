use crate::Cli;
use spectre_p2p_lib::common::ProtocolError;
use spectre_p2p_lib::pb::spectred_message::Payload;
use spectre_p2p_lib::pb::{SpectredMessage, VersionMessage};
use spectre_p2p_lib::{
    ConnectionInitializer, IncomingRoute, Router, SpectredHandshake, SpectredMessagePayloadType,
};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::Sender;
use tonic::async_trait;
use uuid::Uuid;

pub static ROUTER: RwLock<Option<Arc<Router>>> = RwLock::new(None);

pub struct Initializer {
    cli_args: Arc<Cli>,
    sender: Sender<SpectredMessage>,
}

impl Initializer {
    pub fn new(cli_args: Arc<Cli>, sender: Sender<SpectredMessage>) -> Self {
        Initializer { cli_args, sender }
    }
}

#[async_trait]
impl ConnectionInitializer for Initializer {
    async fn initialize_connection(&self, router: Arc<Router>) -> Result<(), ProtocolError> {
        ROUTER.write().unwrap().replace(router.clone());
        let mut handshake = SpectredHandshake::new(&router);
        router.start();
        let version_msg = handshake
            .handshake(build_dummy_version_message(self.cli_args.clone()))
            .await?;
        self.sender
            .send(SpectredMessage {
                request_id: 0,
                response_id: 0,
                payload: Some(Payload::Version(version_msg)),
            })
            .await
            .unwrap();

        let mut incoming_route = subscribe_all(&router);
        let sender = self.sender.clone();
        tokio::spawn(async move {
            while let Some(msg) = incoming_route.recv().await {
                if sender.send(msg).await.is_err() {
                    break;
                }
            }
        });

        handshake.exchange_ready_messages().await?;
        Ok(())
    }
}

fn build_dummy_version_message(cli_args: Arc<Cli>) -> VersionMessage {
    VersionMessage {
        protocol_version: 6,
        services: 0,
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64,
        address: None,
        id: Vec::from(Uuid::new_v4().as_bytes()),
        user_agent: "sp2p".to_string(),
        disable_relay_tx: true,
        subnetwork_id: None,
        network: format!("spectre-{}", cli_args.network.to_lowercase()),
    }
}

fn subscribe_all(router: &Arc<Router>) -> IncomingRoute {
    router.subscribe(vec![
        SpectredMessagePayloadType::Addresses,
        SpectredMessagePayloadType::Block,
        SpectredMessagePayloadType::Transaction,
        SpectredMessagePayloadType::BlockLocator,
        SpectredMessagePayloadType::RequestAddresses,
        SpectredMessagePayloadType::RequestRelayBlocks,
        SpectredMessagePayloadType::RequestTransactions,
        SpectredMessagePayloadType::IbdBlock,
        SpectredMessagePayloadType::InvRelayBlock,
        SpectredMessagePayloadType::InvTransactions,
        SpectredMessagePayloadType::Ping,
        SpectredMessagePayloadType::Pong,
        // SpectredMessagePayloadType::Verack,
        // SpectredMessagePayloadType::Version,
        SpectredMessagePayloadType::TransactionNotFound,
        SpectredMessagePayloadType::Reject,
        SpectredMessagePayloadType::PruningPointUtxoSetChunk,
        SpectredMessagePayloadType::RequestIbdBlocks,
        SpectredMessagePayloadType::UnexpectedPruningPoint,
        SpectredMessagePayloadType::IbdBlockLocator,
        SpectredMessagePayloadType::IbdBlockLocatorHighestHash,
        SpectredMessagePayloadType::RequestNextPruningPointUtxoSetChunk,
        SpectredMessagePayloadType::DonePruningPointUtxoSetChunks,
        SpectredMessagePayloadType::IbdBlockLocatorHighestHashNotFound,
        SpectredMessagePayloadType::BlockWithTrustedData,
        SpectredMessagePayloadType::DoneBlocksWithTrustedData,
        SpectredMessagePayloadType::RequestPruningPointAndItsAnticone,
        SpectredMessagePayloadType::BlockHeaders,
        SpectredMessagePayloadType::RequestNextHeaders,
        SpectredMessagePayloadType::DoneHeaders,
        SpectredMessagePayloadType::RequestPruningPointUtxoSet,
        SpectredMessagePayloadType::RequestHeaders,
        SpectredMessagePayloadType::RequestBlockLocator,
        SpectredMessagePayloadType::PruningPoints,
        SpectredMessagePayloadType::RequestPruningPointProof,
        SpectredMessagePayloadType::PruningPointProof,
        // SpectredMessagePayloadType::Ready,
        SpectredMessagePayloadType::BlockWithTrustedDataV4,
        SpectredMessagePayloadType::TrustedData,
        SpectredMessagePayloadType::RequestIbdChainBlockLocator,
        SpectredMessagePayloadType::IbdChainBlockLocator,
        SpectredMessagePayloadType::RequestAntipast,
        SpectredMessagePayloadType::RequestNextPruningPointAndItsAnticoneBlocks,
    ])
}
