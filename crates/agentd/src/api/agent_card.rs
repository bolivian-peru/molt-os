use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::state::SharedState;

/// EIP-8004 Agent Card — identity + capability discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCard {
    #[serde(rename = "type")]
    pub card_type: String,
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    pub services: Vec<AgentService>,
    pub active: bool,
    #[serde(rename = "supportedTrust")]
    pub supported_trust: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentService {
    pub name: String,
    pub endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

impl Default for AgentCard {
    fn default() -> Self {
        Self {
            card_type: "https://eips.ethereum.org/EIPS/eip-8004#registration-v1".to_string(),
            name: "osModa".to_string(),
            description: "AI-native OS agent — full system access with auditable safety".to_string(),
            image: None,
            services: vec![
                AgentService {
                    name: "mcp".to_string(),
                    endpoint: "unix:///run/osmoda/agentd.sock".to_string(),
                    version: Some("1.0".to_string()),
                },
                AgentService {
                    name: "wallet.sign".to_string(),
                    endpoint: "unix:///run/osmoda/keyd.sock".to_string(),
                    version: Some("1.0".to_string()),
                },
                AgentService {
                    name: "safeswitch".to_string(),
                    endpoint: "unix:///run/osmoda/watch.sock".to_string(),
                    version: Some("1.0".to_string()),
                },
            ],
            active: true,
            supported_trust: vec!["eip-8004".to_string(), "hash-chain-ledger".to_string()],
        }
    }
}

/// GET /agent/card — serve the agent's identity card.
pub async fn agent_card_handler(
    State(state): State<SharedState>,
) -> Json<AgentCard> {
    // Try to load from disk
    let card_path = std::path::Path::new(&state.state_dir).join("agent-card.json");
    if card_path.exists() {
        if let Ok(data) = std::fs::read_to_string(&card_path) {
            if let Ok(card) = serde_json::from_str::<AgentCard>(&data) {
                return Json(card);
            }
        }
    }
    Json(AgentCard::default())
}

/// Request body for generating a new agent card.
#[derive(Debug, Deserialize)]
pub struct GenerateCardRequest {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub services: Vec<AgentService>,
}

/// POST /agent/card/generate — generate and store a new agent card.
pub async fn agent_card_generate_handler(
    State(state): State<SharedState>,
    Json(body): Json<GenerateCardRequest>,
) -> Result<Json<AgentCard>, axum::http::StatusCode> {
    let card = AgentCard {
        card_type: "https://eips.ethereum.org/EIPS/eip-8004#registration-v1".to_string(),
        name: body.name,
        description: body.description,
        image: None,
        services: if body.services.is_empty() {
            AgentCard::default().services
        } else {
            body.services
        },
        active: true,
        supported_trust: vec!["eip-8004".to_string(), "hash-chain-ledger".to_string()],
    };

    // Store to disk
    let card_path = std::path::Path::new(&state.state_dir).join("agent-card.json");
    let card_json = serde_json::to_string_pretty(&card).map_err(|e| {
        tracing::error!(error = %e, "failed to serialize agent card");
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;
    std::fs::write(&card_path, &card_json).map_err(|e| {
        tracing::error!(error = %e, "failed to write agent card");
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Log to ledger
    let ledger = state.ledger.lock().await;
    if let Err(e) = ledger.append(
        "agent.card.generate",
        "agentd",
        &json!({"name": card.name, "services": card.services.len()}).to_string(),
    ) {
        tracing::warn!(error = %e, "failed to log agent card generation");
    }

    tracing::info!(name = %card.name, "agent card generated");
    Ok(Json(card))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_card_serialization() {
        let card = AgentCard::default();
        let json = serde_json::to_string(&card).unwrap();
        assert!(json.contains("eip-8004"));
        assert!(json.contains("osModa"));

        // Roundtrip
        let parsed: AgentCard = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "osModa");
        assert_eq!(parsed.card_type, "https://eips.ethereum.org/EIPS/eip-8004#registration-v1");
    }
}
