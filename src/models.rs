use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct NodeData {
    pub ip: String,
    pub metadata: NodeMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loc: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct NodeMetadata {
    pub protocol_version: u32,
    pub network: String,
    pub services: u64,
    pub timestamp: String,
    pub id: String,
    pub user_agent: String,
    pub disable_relay_tx: bool,
}
