use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::policy::PolicyDecision;
use crate::receipt::WalletReceipt;
use crate::signer::Chain;
use crate::KeydState;

type SharedState = Arc<KeydState>;

// ── POST /wallet/create ──

#[derive(Debug, Deserialize)]
pub struct CreateWalletRequest {
    pub chain: Chain,
    pub label: String,
}

#[derive(Debug, Serialize)]
pub struct CreateWalletResponse {
    pub id: String,
    pub chain: Chain,
    pub address: String,
    pub label: String,
}

pub async fn wallet_create_handler(
    State(state): State<SharedState>,
    Json(body): Json<CreateWalletRequest>,
) -> Result<Json<CreateWalletResponse>, axum::http::StatusCode> {
    let mut signer = state.signer.lock().await;

    let wallet = signer.create_wallet(body.chain, &body.label).map_err(|e| {
        tracing::error!(error = %e, "failed to create wallet");
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Log receipt
    state.receipt_logger.log_receipt(&WalletReceipt {
        wallet_id: wallet.id.clone(),
        action: "create".to_string(),
        chain: wallet.chain.to_string(),
        to: None,
        amount: None,
        policy_decision: "allowed".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    }).await;

    Ok(Json(CreateWalletResponse {
        id: wallet.id,
        chain: wallet.chain,
        address: wallet.address,
        label: wallet.label,
    }))
}

// ── GET /wallet/list ──

pub async fn wallet_list_handler(
    State(state): State<SharedState>,
) -> Json<Vec<crate::signer::WalletInfo>> {
    let signer = state.signer.lock().await;
    Json(signer.list_wallets())
}

// ── POST /wallet/sign ──

#[derive(Debug, Deserialize)]
pub struct SignRequest {
    pub wallet_id: String,
    pub payload: String, // hex-encoded bytes
}

#[derive(Debug, Serialize)]
pub struct SignResponse {
    pub signature: String, // hex-encoded
    pub wallet_id: String,
    pub policy_decision: String,
}

pub async fn wallet_sign_handler(
    State(state): State<SharedState>,
    Json(body): Json<SignRequest>,
) -> Result<Json<SignResponse>, (axum::http::StatusCode, String)> {
    // Look up wallet chain before signing
    let chain = {
        let signer = state.signer.lock().await;
        signer
            .list_wallets()
            .iter()
            .find(|w| w.id == body.wallet_id)
            .map(|w| w.chain.to_string())
            .ok_or_else(|| {
                (
                    axum::http::StatusCode::NOT_FOUND,
                    "wallet not found".to_string(),
                )
            })?
    };

    // Check policy
    let decision = {
        let mut policy = state.policy.lock().await;
        policy.check_sign()
    };

    match &decision {
        PolicyDecision::Denied { reason } => {
            state.receipt_logger.log_receipt(&WalletReceipt {
                wallet_id: body.wallet_id.clone(),
                action: "sign".to_string(),
                chain: chain.clone(),
                to: None,
                amount: None,
                policy_decision: format!("denied: {reason}"),
                timestamp: chrono::Utc::now().to_rfc3339(),
            }).await;

            return Err((
                axum::http::StatusCode::FORBIDDEN,
                serde_json::json!({"error": "policy_denied", "reason": reason}).to_string(),
            ));
        }
        PolicyDecision::Allowed => {}
    }

    let message = hex::decode(&body.payload).map_err(|e| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            format!("invalid hex payload: {e}"),
        )
    })?;

    let mut signer = state.signer.lock().await;
    let signature = signer
        .sign_message(&body.wallet_id, &message)
        .await
        .map_err(|e| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("sign failed: {e}"),
            )
        })?;

    state.receipt_logger.log_receipt(&WalletReceipt {
        wallet_id: body.wallet_id.clone(),
        action: "sign".to_string(),
        chain,
        to: None,
        amount: None,
        policy_decision: "allowed".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    }).await;

    Ok(Json(SignResponse {
        signature: hex::encode(&signature),
        wallet_id: body.wallet_id,
        policy_decision: "allowed".to_string(),
    }))
}

// ── POST /wallet/send ──

#[derive(Debug, Deserialize)]
pub struct SendRequest {
    pub wallet_id: String,
    pub to: String,
    pub amount: String,
}

#[derive(Debug, Serialize)]
pub struct SendResponse {
    pub signed_tx: String, // hex-encoded signed transaction
    pub wallet_id: String,
    pub policy_decision: String,
    pub note: String,
}

pub async fn wallet_send_handler(
    State(state): State<SharedState>,
    Json(body): Json<SendRequest>,
) -> Result<Json<SendResponse>, (axum::http::StatusCode, String)> {
    // Determine chain from wallet
    let chain = {
        let signer = state.signer.lock().await;
        let wallets = signer.list_wallets();
        wallets
            .iter()
            .find(|w| w.id == body.wallet_id)
            .map(|w| w.chain.to_string())
            .ok_or_else(|| {
                (
                    axum::http::StatusCode::NOT_FOUND,
                    "wallet not found".to_string(),
                )
            })?
    };

    // Check policy (string-based decimal, no float precision issues)
    let decision = {
        let mut policy = state.policy.lock().await;
        policy.check_send(&chain, &body.amount, &body.to)
    };

    match &decision {
        PolicyDecision::Denied { reason } => {
            state.receipt_logger.log_receipt(&WalletReceipt {
                wallet_id: body.wallet_id.clone(),
                action: "send".to_string(),
                chain: chain.clone(),
                to: Some(body.to.clone()),
                amount: Some(body.amount.clone()),
                policy_decision: format!("denied: {reason}"),
                timestamp: chrono::Utc::now().to_rfc3339(),
            }).await;

            return Err((
                axum::http::StatusCode::FORBIDDEN,
                serde_json::json!({"error": "policy_denied", "reason": reason}).to_string(),
            ));
        }
        PolicyDecision::Allowed => {}
    }

    // Build a simple message representing the transaction intent
    // (keyd has no network, so we can't broadcast — we sign the intent)
    let tx_intent = format!("send:{chain}:{to}:{amount}", to = body.to, amount = body.amount);
    let mut signer = state.signer.lock().await;
    let signature = signer
        .sign_message(&body.wallet_id, tx_intent.as_bytes())
        .await
        .map_err(|e| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("sign failed: {e}"),
            )
        })?;

    state.receipt_logger.log_receipt(&WalletReceipt {
        wallet_id: body.wallet_id.clone(),
        action: "send".to_string(),
        chain: chain.clone(),
        to: Some(body.to.clone()),
        amount: Some(body.amount.clone()),
        policy_decision: "allowed".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    }).await;

    Ok(Json(SendResponse {
        signed_tx: hex::encode(&signature),
        wallet_id: body.wallet_id,
        policy_decision: "allowed".to_string(),
        note: "keyd has no network access — signed tx returned for external broadcast".to_string(),
    }))
}

// ── POST /wallet/build_tx ──

#[derive(Debug, Deserialize)]
pub struct BuildTxRequest {
    pub wallet_id: String,
    pub tx_type: String, // "transfer"
    pub to: String,
    pub amount: String,
    #[serde(default)]
    pub chain_params: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct BuildTxResponse {
    pub signed_tx: String,
    pub tx_hash: Option<String>,
    pub from: String,
    pub to: String,
    pub amount: String,
    pub chain: String,
    pub policy_decision: String,
}

pub async fn wallet_build_tx_handler(
    State(state): State<SharedState>,
    Json(body): Json<BuildTxRequest>,
) -> Result<Json<BuildTxResponse>, (axum::http::StatusCode, String)> {
    // Determine chain from wallet
    let wallet_chain = {
        let signer = state.signer.lock().await;
        let wallets = signer.list_wallets();
        wallets
            .iter()
            .find(|w| w.id == body.wallet_id)
            .map(|w| w.chain)
            .ok_or_else(|| {
                (
                    axum::http::StatusCode::NOT_FOUND,
                    "wallet not found".to_string(),
                )
            })?
    };

    // Check policy
    let decision = {
        let mut policy = state.policy.lock().await;
        policy.check_send(&wallet_chain.to_string(), &body.amount, &body.to)
    };

    match &decision {
        PolicyDecision::Denied { reason } => {
            state.receipt_logger.log_receipt(&WalletReceipt {
                wallet_id: body.wallet_id.clone(),
                action: "build_tx".to_string(),
                chain: wallet_chain.to_string(),
                to: Some(body.to.clone()),
                amount: Some(body.amount.clone()),
                policy_decision: format!("denied: {reason}"),
                timestamp: chrono::Utc::now().to_rfc3339(),
            }).await;

            return Err((
                axum::http::StatusCode::FORBIDDEN,
                serde_json::json!({"error": "policy_denied", "reason": reason}).to_string(),
            ));
        }
        PolicyDecision::Allowed => {}
    }

    // Load key bytes
    let key_bytes = {
        let mut signer = state.signer.lock().await;
        signer.load_key_bytes_pub(&body.wallet_id).map_err(|e| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to load key: {e}"),
            )
        })?
    };

    let result = match wallet_chain {
        Chain::Ethereum => {
            let params = crate::tx_eth::EthTxParams {
                chain_id: body.chain_params.get("chain_id")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1),
                nonce: body.chain_params.get("nonce")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                to: body.to.clone(),
                value: body.amount.clone(),
                max_fee_per_gas: body.chain_params.get("max_fee_per_gas")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(30_000_000_000),
                max_priority_fee_per_gas: body.chain_params.get("max_priority_fee_per_gas")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1_000_000_000),
                gas_limit: body.chain_params.get("gas_limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(21_000),
                data: body.chain_params.get("data")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            };

            let tx_result = crate::tx_eth::build_and_sign_eip1559(&key_bytes, &params)
                .map_err(|e| {
                    (
                        axum::http::StatusCode::BAD_REQUEST,
                        format!("ETH tx build failed: {e}"),
                    )
                })?;

            BuildTxResponse {
                signed_tx: tx_result.signed_tx,
                tx_hash: Some(tx_result.tx_hash),
                from: tx_result.from,
                to: tx_result.to,
                amount: tx_result.value,
                chain: "ethereum".to_string(),
                policy_decision: "allowed".to_string(),
            }
        }
        Chain::Solana => {
            let recent_blockhash = body.chain_params.get("recent_blockhash")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    (
                        axum::http::StatusCode::BAD_REQUEST,
                        "Solana tx requires chain_params.recent_blockhash".to_string(),
                    )
                })?;

            let lamports: u64 = body.amount.parse().map_err(|_| {
                (
                    axum::http::StatusCode::BAD_REQUEST,
                    "amount must be integer lamports for Solana".to_string(),
                )
            })?;

            let params = crate::tx_sol::SolTxParams {
                to: body.to.clone(),
                lamports,
                recent_blockhash: recent_blockhash.to_string(),
            };

            let tx_result = crate::tx_sol::build_and_sign_transfer(&key_bytes, &params)
                .map_err(|e| {
                    (
                        axum::http::StatusCode::BAD_REQUEST,
                        format!("SOL tx build failed: {e}"),
                    )
                })?;

            BuildTxResponse {
                signed_tx: tx_result.signed_tx,
                tx_hash: Some(tx_result.signature),
                from: tx_result.from,
                to: tx_result.to,
                amount: tx_result.lamports.to_string(),
                chain: "solana".to_string(),
                policy_decision: "allowed".to_string(),
            }
        }
    };

    state.receipt_logger.log_receipt(&WalletReceipt {
        wallet_id: body.wallet_id.clone(),
        action: "build_tx".to_string(),
        chain: wallet_chain.to_string(),
        to: Some(body.to.clone()),
        amount: Some(body.amount.clone()),
        policy_decision: "allowed".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    }).await;

    Ok(Json(result))
}

// ── DELETE /wallet/delete ──

#[derive(Debug, Deserialize)]
pub struct DeleteWalletRequest {
    pub wallet_id: String,
}

pub async fn wallet_delete_handler(
    State(state): State<SharedState>,
    Json(body): Json<DeleteWalletRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let mut signer = state.signer.lock().await;

    signer.delete_wallet(&body.wallet_id).map_err(|e| {
        (
            axum::http::StatusCode::NOT_FOUND,
            format!("delete failed: {e}"),
        )
    })?;

    state.receipt_logger.log_receipt(&WalletReceipt {
        wallet_id: body.wallet_id.clone(),
        action: "delete".to_string(),
        chain: "n/a".to_string(),
        to: None,
        amount: None,
        policy_decision: "allowed".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    }).await;

    Ok(Json(serde_json::json!({"deleted": body.wallet_id})))
}

// ── GET /health ──

#[derive(Debug, Serialize)]
pub struct KeydHealthResponse {
    pub status: String,
    pub wallet_count: usize,
    pub policy_loaded: bool,
}

pub async fn health_handler(State(state): State<SharedState>) -> Json<KeydHealthResponse> {
    let signer = state.signer.lock().await;
    let policy = state.policy.lock().await;

    Json(KeydHealthResponse {
        status: "ok".to_string(),
        wallet_count: signer.wallet_count(),
        policy_loaded: policy.is_loaded(),
    })
}
