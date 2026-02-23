use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub action: String,
    pub max_amount: Option<String>,
    pub period: Option<String>,
    pub allowed_destinations: Option<Vec<String>>,
    pub chain: Option<String>,
    pub max_per_day: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyFile {
    pub rules: Vec<PolicyRule>,
}

impl Default for PolicyFile {
    fn default() -> Self {
        Self {
            rules: vec![
                PolicyRule {
                    action: "send".to_string(),
                    max_amount: Some("1.0".to_string()),
                    period: Some("daily".to_string()),
                    allowed_destinations: None,
                    chain: Some("ethereum".to_string()),
                    max_per_day: Some(10),
                },
                PolicyRule {
                    action: "send".to_string(),
                    max_amount: Some("10.0".to_string()),
                    period: Some("daily".to_string()),
                    allowed_destinations: None,
                    chain: Some("solana".to_string()),
                    max_per_day: Some(20),
                },
                PolicyRule {
                    action: "sign".to_string(),
                    max_amount: None,
                    period: Some("daily".to_string()),
                    allowed_destinations: None,
                    chain: None,
                    max_per_day: Some(100),
                },
            ],
        }
    }
}

#[derive(Debug, Clone)]
pub enum PolicyDecision {
    Allowed,
    Denied { reason: String },
}

/// Fixed-point amount with 18 decimal places (same precision as wei).
/// Stored as u128 to handle amounts up to ~340 undecillion base units.
#[derive(Debug, Clone, Copy, Default)]
struct FixedAmount(u128);

const DECIMALS: u32 = 18;

impl FixedAmount {
    /// Parse a decimal string like "1.5" into fixed-point representation.
    /// Supports up to 18 decimal places.
    fn from_str(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }

        let (integer_part, decimal_part) = if let Some(dot_pos) = s.find('.') {
            (&s[..dot_pos], &s[dot_pos + 1..])
        } else {
            (s, "")
        };

        let integer: u128 = integer_part.parse().ok()?;
        let decimal_digits = decimal_part.len() as u32;

        if decimal_digits > DECIMALS {
            return None; // too many decimal places
        }

        let decimal: u128 = if decimal_part.is_empty() {
            0
        } else {
            decimal_part.parse().ok()?
        };

        let scale = 10u128.pow(DECIMALS);
        let decimal_scale = 10u128.pow(DECIMALS - decimal_digits);

        Some(FixedAmount(
            integer.checked_mul(scale)?.checked_add(decimal.checked_mul(decimal_scale)?)?,
        ))
    }

    fn checked_add(self, other: Self) -> Option<Self> {
        self.0.checked_add(other.0).map(FixedAmount)
    }
}

impl std::fmt::Display for FixedAmount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let scale = 10u128.pow(DECIMALS);
        let integer = self.0 / scale;
        let decimal = self.0 % scale;
        // Trim trailing zeros from decimal part
        if decimal == 0 {
            write!(f, "{integer}.0")
        } else {
            let dec_str = format!("{decimal:018}");
            let trimmed = dec_str.trim_end_matches('0');
            write!(f, "{integer}.{trimmed}")
        }
    }
}

/// Tracks daily usage counters for policy enforcement.
struct DailyCounters {
    date: String,
    send_counts: HashMap<String, u32>,
    send_amounts: HashMap<String, FixedAmount>,
    sign_count: u32,
}

impl DailyCounters {
    fn new() -> Self {
        Self {
            date: today(),
            send_counts: HashMap::new(),
            send_amounts: HashMap::new(),
            sign_count: 0,
        }
    }

    fn reset_if_new_day(&mut self) {
        let now = today();
        if now != self.date {
            self.date = now;
            self.send_counts.clear();
            self.send_amounts.clear();
            self.sign_count = 0;
        }
    }
}

fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

pub struct PolicyEngine {
    policy: PolicyFile,
    counters: DailyCounters,
}

impl PolicyEngine {
    pub fn new(policy_path: &Path) -> Result<Self> {
        let policy = if policy_path.exists() {
            let data =
                std::fs::read_to_string(policy_path).context("failed to read policy file")?;
            serde_json::from_str(&data).context("failed to parse policy file")?
        } else {
            let default = PolicyFile::default();
            let data = serde_json::to_string_pretty(&default)?;
            if let Some(parent) = policy_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(policy_path, &data)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(policy_path, std::fs::Permissions::from_mode(0o600))?;
            }
            tracing::info!("created default policy file at {}", policy_path.display());
            default
        };

        Ok(Self {
            policy,
            counters: DailyCounters::new(),
        })
    }

    /// Check whether a send operation is allowed by policy.
    /// `amount_str` is a decimal string like "0.5" or "10.0".
    pub fn check_send(&mut self, chain: &str, amount_str: &str, destination: &str) -> PolicyDecision {
        self.counters.reset_if_new_day();

        let amount = match FixedAmount::from_str(amount_str) {
            Some(a) => a,
            None => {
                return PolicyDecision::Denied {
                    reason: format!("invalid amount: {amount_str}"),
                };
            }
        };

        for rule in &self.policy.rules {
            if rule.action != "send" {
                continue;
            }
            if let Some(ref rule_chain) = rule.chain {
                if rule_chain != chain {
                    continue;
                }
            }

            // Check destination allowlist
            if let Some(ref allowed) = rule.allowed_destinations {
                if !allowed.contains(&destination.to_string()) {
                    return PolicyDecision::Denied {
                        reason: format!("destination {destination} not in allowlist"),
                    };
                }
            }

            // Check daily count
            if let Some(max) = rule.max_per_day {
                let count = self.counters.send_counts.get(chain).copied().unwrap_or(0);
                if count >= max {
                    return PolicyDecision::Denied {
                        reason: format!("daily send limit reached ({max}) for {chain}"),
                    };
                }
            }

            // Check daily amount (fixed-point, no float precision issues)
            if let Some(ref max_str) = rule.max_amount {
                if let Some(max_amount) = FixedAmount::from_str(max_str) {
                    let current = self.counters.send_amounts.get(chain).copied().unwrap_or_default();
                    match current.checked_add(amount) {
                        Some(total) if total.0 > max_amount.0 => {
                            return PolicyDecision::Denied {
                                reason: format!(
                                    "daily amount limit exceeded: {current} + {amount_str} > {max_amount} {chain}"
                                ),
                            };
                        }
                        None => {
                            return PolicyDecision::Denied {
                                reason: "amount overflow".to_string(),
                            };
                        }
                        _ => {}
                    }
                }
            }
        }

        // Record usage
        *self
            .counters
            .send_counts
            .entry(chain.to_string())
            .or_insert(0) += 1;
        let current = self
            .counters
            .send_amounts
            .entry(chain.to_string())
            .or_insert(FixedAmount(0));
        if let Some(new_total) = current.checked_add(amount) {
            *current = new_total;
        }

        PolicyDecision::Allowed
    }

    pub fn check_sign(&mut self) -> PolicyDecision {
        self.counters.reset_if_new_day();

        for rule in &self.policy.rules {
            if rule.action != "sign" {
                continue;
            }
            if let Some(max) = rule.max_per_day {
                if self.counters.sign_count >= max {
                    return PolicyDecision::Denied {
                        reason: format!("daily sign limit reached ({max})"),
                    };
                }
            }
        }

        self.counters.sign_count += 1;
        PolicyDecision::Allowed
    }

    pub fn is_loaded(&self) -> bool {
        !self.policy.rules.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixed_amount_parse() {
        assert_eq!(FixedAmount::from_str("1.0").unwrap().0, 10u128.pow(18));
        assert_eq!(FixedAmount::from_str("0.5").unwrap().0, 5 * 10u128.pow(17));
        assert_eq!(FixedAmount::from_str("10").unwrap().0, 10 * 10u128.pow(18));
        assert_eq!(
            FixedAmount::from_str("0.000000000000000001").unwrap().0,
            1 // 1 wei
        );
        assert!(FixedAmount::from_str("").is_none());
        assert!(FixedAmount::from_str("abc").is_none());
    }

    #[test]
    fn test_fixed_amount_display() {
        let a = FixedAmount::from_str("1.5").unwrap();
        assert_eq!(a.to_string(), "1.5");
        let b = FixedAmount::from_str("10.0").unwrap();
        assert_eq!(b.to_string(), "10.0");
        let c = FixedAmount::from_str("0.123").unwrap();
        assert_eq!(c.to_string(), "0.123");
    }

    #[test]
    fn test_fixed_amount_precision() {
        // Classic float failure: 0.1 + 0.2 != 0.3 in f64
        let a = FixedAmount::from_str("0.1").unwrap();
        let b = FixedAmount::from_str("0.2").unwrap();
        let c = FixedAmount::from_str("0.3").unwrap();
        assert_eq!(a.checked_add(b).unwrap().0, c.0);
    }

    #[test]
    fn test_default_policy_allows_small_send() {
        let dir = tempfile::tempdir().unwrap();
        let policy_path = dir.path().join("policy.json");
        let mut engine = PolicyEngine::new(&policy_path).unwrap();

        match engine.check_send("ethereum", "0.5", "0xabc") {
            PolicyDecision::Allowed => {}
            PolicyDecision::Denied { reason } => panic!("should be allowed: {reason}"),
        }
    }

    #[test]
    fn test_policy_denies_over_limit() {
        let dir = tempfile::tempdir().unwrap();
        let policy_path = dir.path().join("policy.json");
        let mut engine = PolicyEngine::new(&policy_path).unwrap();

        // First send: 0.8 ETH — allowed
        assert!(matches!(
            engine.check_send("ethereum", "0.8", "0xabc"),
            PolicyDecision::Allowed
        ));
        // Second send: 0.5 ETH — would exceed 1.0 daily limit
        assert!(matches!(
            engine.check_send("ethereum", "0.5", "0xabc"),
            PolicyDecision::Denied { .. }
        ));
    }

    #[test]
    fn test_sign_limit() {
        let dir = tempfile::tempdir().unwrap();
        let policy_path = dir.path().join("policy.json");
        let mut engine = PolicyEngine::new(&policy_path).unwrap();

        for _ in 0..100 {
            assert!(matches!(engine.check_sign(), PolicyDecision::Allowed));
        }
        // 101st should be denied
        assert!(matches!(engine.check_sign(), PolicyDecision::Denied { .. }));
    }

    #[test]
    fn test_policy_destination_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        let policy_path = dir.path().join("policy.json");
        let policy = PolicyFile {
            rules: vec![PolicyRule {
                action: "send".to_string(),
                max_amount: Some("100.0".to_string()),
                period: Some("daily".to_string()),
                allowed_destinations: Some(vec!["0xallowed".to_string()]),
                chain: Some("ethereum".to_string()),
                max_per_day: Some(100),
            }],
        };
        std::fs::write(&policy_path, serde_json::to_string(&policy).unwrap()).unwrap();
        let mut engine = PolicyEngine::new(&policy_path).unwrap();

        assert!(matches!(
            engine.check_send("ethereum", "0.1", "0xallowed"),
            PolicyDecision::Allowed
        ));
        assert!(matches!(
            engine.check_send("ethereum", "0.1", "0xblocked"),
            PolicyDecision::Denied { .. }
        ));
    }

    #[test]
    fn test_policy_invalid_amount() {
        let dir = tempfile::tempdir().unwrap();
        let policy_path = dir.path().join("policy.json");
        let mut engine = PolicyEngine::new(&policy_path).unwrap();

        assert!(matches!(
            engine.check_send("ethereum", "not_a_number", "0xabc"),
            PolicyDecision::Denied { .. }
        ));
    }
}
