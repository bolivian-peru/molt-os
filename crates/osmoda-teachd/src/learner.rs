use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::knowledge::{
    insert_knowledge_doc, list_observations, upsert_pattern, KnowledgeDoc, Observation, Pattern,
    PatternType,
};
use crate::TeachdState;

/// Background loop: analyzes observations for patterns every 5 minutes.
pub async fn learner_loop(state: Arc<Mutex<TeachdState>>, cancel: CancellationToken) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(300));

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("learner loop shutting down");
                break;
            }
            _ = interval.tick() => {
                if let Err(e) = run_learning_cycle(&state).await {
                    tracing::warn!(error = %e, "learning cycle failed");
                }
            }
        }
    }
}

async fn run_learning_cycle(state: &Arc<Mutex<TeachdState>>) -> anyhow::Result<()> {
    let st = state.lock().await;

    // Get recent observations (last hour)
    let since = (Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
    let observations = list_observations(&st.db, None, Some(&since), 500)?;

    if observations.is_empty() {
        return Ok(());
    }

    // Detect different pattern types
    let mut patterns = Vec::new();
    patterns.extend(detect_recurring_failures(&observations));
    patterns.extend(detect_resource_trends(&observations));
    patterns.extend(detect_anomalies(&observations));
    patterns.extend(detect_correlations(&observations));

    let receipt_logger = st.receipt_logger.clone();

    for pattern in &patterns {
        upsert_pattern(&st.db, pattern)?;

        if pattern.confidence > 0.7 {
            generate_knowledge_from_pattern(&st.db, pattern)?;
            drop(receipt_logger.log_event(
                "pattern.detected",
                &pattern.name,
                &format!(
                    "type={}, confidence={:.2}, occurrences={}",
                    pattern.pattern_type, pattern.confidence, pattern.occurrence_count
                ),
            ));
        }
    }

    if !patterns.is_empty() {
        tracing::info!(count = patterns.len(), "learning cycle detected patterns");
    }

    Ok(())
}

/// Detect recurring service failures: same service failing 3+ times.
fn detect_recurring_failures(observations: &[Observation]) -> Vec<Pattern> {
    let mut failure_counts: HashMap<String, Vec<&Observation>> = HashMap::new();

    for obs in observations {
        if obs.source == "service" {
            if let Some(names) = obs.data.get("failed_names").and_then(|v| v.as_array()) {
                for name in names {
                    if let Some(n) = name.as_str() {
                        failure_counts
                            .entry(n.to_string())
                            .or_default()
                            .push(obs);
                    }
                }
            }
        }
        if obs.source == "journal" {
            if let Some(errors) = obs.data.get("errors").and_then(|v| v.as_array()) {
                for err in errors {
                    if let Some(ident) = err.get("identifier").and_then(|v| v.as_str()) {
                        failure_counts
                            .entry(ident.to_string())
                            .or_default()
                            .push(obs);
                    }
                }
            }
        }
    }

    failure_counts
        .into_iter()
        .filter(|(_, obs)| obs.len() >= 3)
        .map(|(name, obs)| {
            let count = obs.len() as u32;
            let confidence = (count as f64 / 10.0).min(1.0);
            let obs_ids: Vec<String> = obs.iter().map(|o| o.id.clone()).collect();
            let first = obs.iter().map(|o| o.ts).min().unwrap_or_else(Utc::now);
            let last = obs.iter().map(|o| o.ts).max().unwrap_or_else(Utc::now);

            Pattern {
                id: format!("pat-recurring-{}", name),
                name: format!("{} recurring failures", name),
                pattern_type: PatternType::Recurring,
                confidence,
                observations: obs_ids,
                first_seen: first,
                last_seen: last,
                occurrence_count: count,
            }
        })
        .collect()
}

/// Detect monotonic resource trends (memory/CPU increase over the hour).
fn detect_resource_trends(observations: &[Observation]) -> Vec<Pattern> {
    let mut patterns = Vec::new();

    // Check memory trend
    let mem_obs: Vec<&Observation> = observations
        .iter()
        .filter(|o| o.source == "memory")
        .collect();

    if mem_obs.len() >= 6 {
        // Need at least 6 data points (3 minutes of 30s intervals)
        let usage_values: Vec<f64> = mem_obs
            .iter()
            .filter_map(|o| {
                let total = o.data.get("total_kb")?.as_f64()?;
                let available = o.data.get("available_kb")?.as_f64()?;
                if total > 0.0 {
                    Some((total - available) / total * 100.0)
                } else {
                    None
                }
            })
            .collect();

        if let Some(trend) = detect_monotonic_trend(&usage_values) {
            if trend > 0.0 {
                let obs_ids: Vec<String> = mem_obs.iter().map(|o| o.id.clone()).collect();
                let confidence = (trend / 5.0).min(1.0); // Higher trend = higher confidence
                patterns.push(Pattern {
                    id: "pat-trend-memory-increase".to_string(),
                    name: "memory usage increasing".to_string(),
                    pattern_type: PatternType::Trend,
                    confidence,
                    observations: obs_ids,
                    first_seen: mem_obs.first().map(|o| o.ts).unwrap_or_else(Utc::now),
                    last_seen: mem_obs.last().map(|o| o.ts).unwrap_or_else(Utc::now),
                    occurrence_count: mem_obs.len() as u32,
                });
            }
        }
    }

    patterns
}

/// Detect anomalies: sudden spikes (>2 std deviations from rolling average).
fn detect_anomalies(observations: &[Observation]) -> Vec<Pattern> {
    let mut patterns = Vec::new();

    // Check for journal error spikes
    let journal_obs: Vec<&Observation> = observations
        .iter()
        .filter(|o| o.source == "journal")
        .collect();

    if journal_obs.len() >= 3 {
        let error_counts: Vec<f64> = journal_obs
            .iter()
            .filter_map(|o| o.data.get("error_count")?.as_f64())
            .collect();

        if !error_counts.is_empty() {
            let mean = error_counts.iter().sum::<f64>() / error_counts.len() as f64;
            let variance = error_counts
                .iter()
                .map(|v| (v - mean).powi(2))
                .sum::<f64>()
                / error_counts.len() as f64;
            let std_dev = variance.sqrt();

            // Check if any recent count is >2 std devs above mean
            if let Some(&last_count) = error_counts.last() {
                if std_dev > 0.0 && last_count > mean + 2.0 * std_dev {
                    let obs_ids: Vec<String> = journal_obs.iter().map(|o| o.id.clone()).collect();
                    patterns.push(Pattern {
                        id: "pat-anomaly-error-spike".to_string(),
                        name: "journal error spike detected".to_string(),
                        pattern_type: PatternType::Anomaly,
                        confidence: 0.8,
                        observations: obs_ids,
                        first_seen: journal_obs
                            .first()
                            .map(|o| o.ts)
                            .unwrap_or_else(Utc::now),
                        last_seen: journal_obs
                            .last()
                            .map(|o| o.ts)
                            .unwrap_or_else(Utc::now),
                        occurrence_count: 1,
                    });
                }
            }
        }
    }

    patterns
}

/// Detect correlations: e.g., high CPU within 60s of a service failure.
fn detect_correlations(observations: &[Observation]) -> Vec<Pattern> {
    let mut patterns = Vec::new();

    let service_obs: Vec<&Observation> = observations
        .iter()
        .filter(|o| o.source == "service" && o.data.get("failed").and_then(|v| v.as_u64()).unwrap_or(0) > 0)
        .collect();

    let cpu_obs: Vec<&Observation> = observations
        .iter()
        .filter(|o| o.source == "cpu")
        .collect();

    // For each service failure, check if CPU was high within 60 seconds
    for svc in &service_obs {
        for cpu in &cpu_obs {
            let time_diff = (svc.ts - cpu.ts).num_seconds().unsigned_abs();
            if time_diff <= 60 {
                let total = cpu.data.get("total").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let idle = cpu.data.get("idle").and_then(|v| v.as_f64()).unwrap_or(0.0);
                if total > 0.0 {
                    let cpu_usage = (total - idle) / total * 100.0;
                    if cpu_usage > 80.0 {
                        patterns.push(Pattern {
                            id: format!("pat-corr-cpu-svc-{}", svc.id),
                            name: "service failure correlated with high CPU".to_string(),
                            pattern_type: PatternType::Correlation,
                            confidence: 0.75,
                            observations: vec![svc.id.clone(), cpu.id.clone()],
                            first_seen: svc.ts.min(cpu.ts),
                            last_seen: svc.ts.max(cpu.ts),
                            occurrence_count: 1,
                        });
                        break; // One correlation per service observation
                    }
                }
            }
        }
    }

    patterns
}

/// Check if values have a monotonic increasing trend.
/// Returns the average rate of change per data point, or None if insufficient data.
fn detect_monotonic_trend(values: &[f64]) -> Option<f64> {
    if values.len() < 3 {
        return None;
    }

    let mut increasing = 0;
    let mut total_change = 0.0;

    for window in values.windows(2) {
        let diff = window[1] - window[0];
        total_change += diff;
        if diff > 0.0 {
            increasing += 1;
        }
    }

    let pairs = values.len() - 1;
    // At least 60% of pairs should be increasing
    if increasing as f64 / pairs as f64 > 0.6 && total_change > 0.0 {
        Some(total_change / pairs as f64)
    } else {
        None
    }
}

/// Generate a knowledge document from a high-confidence pattern.
fn generate_knowledge_from_pattern(
    db: &rusqlite::Connection,
    pattern: &Pattern,
) -> anyhow::Result<()> {
    let now = Utc::now();
    let (category, content) = match &pattern.pattern_type {
        PatternType::Recurring => (
            "reliability".to_string(),
            format!(
                "## {}\n\n**Pattern**: Recurring failure detected.\n**Occurrences**: {} times\n**First seen**: {}\n**Last seen**: {}\n**Confidence**: {:.0}%\n\n### Recommendation\nInvestigate the root cause of this recurring failure. Consider adding a watcher to auto-restart the affected service.",
                pattern.name, pattern.occurrence_count, pattern.first_seen, pattern.last_seen, pattern.confidence * 100.0
            ),
        ),
        PatternType::Trend => (
            "performance".to_string(),
            format!(
                "## {}\n\n**Pattern**: Resource usage trend detected.\n**Data points**: {}\n**First seen**: {}\n**Last seen**: {}\n**Confidence**: {:.0}%\n\n### Recommendation\nMonitor resource usage and consider scaling or investigating memory leaks.",
                pattern.name, pattern.occurrence_count, pattern.first_seen, pattern.last_seen, pattern.confidence * 100.0
            ),
        ),
        PatternType::Anomaly => (
            "reliability".to_string(),
            format!(
                "## {}\n\n**Pattern**: Anomalous behavior detected.\n**Observations**: {}\n**First seen**: {}\n**Last seen**: {}\n**Confidence**: {:.0}%\n\n### Recommendation\nInvestigate the spike and check system logs for root cause.",
                pattern.name, pattern.occurrence_count, pattern.first_seen, pattern.last_seen, pattern.confidence * 100.0
            ),
        ),
        PatternType::Correlation => (
            "performance".to_string(),
            format!(
                "## {}\n\n**Pattern**: Correlated events detected.\n**Observations**: {}\n**First seen**: {}\n**Last seen**: {}\n**Confidence**: {:.0}%\n\n### Recommendation\nThe correlated events suggest a causal relationship. Investigate if one event triggers the other.",
                pattern.name, pattern.observations.len(), pattern.first_seen, pattern.last_seen, pattern.confidence * 100.0
            ),
        ),
    };

    let tags = vec![
        pattern.pattern_type.to_string(),
        category.clone(),
    ];

    let doc = KnowledgeDoc {
        id: format!("kd-{}", pattern.id),
        title: pattern.name.clone(),
        category,
        content,
        source_patterns: vec![pattern.id.clone()],
        confidence: pattern.confidence,
        created_at: now,
        updated_at: now,
        applied: false,
        tags,
    };

    insert_knowledge_doc(db, &doc)?;
    tracing::info!(
        doc_id = %doc.id,
        title = %doc.title,
        confidence = pattern.confidence,
        "generated knowledge document from pattern"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::Observation;

    #[test]
    fn test_detect_monotonic_trend_increasing() {
        let values = vec![10.0, 12.0, 14.0, 16.0, 18.0, 20.0];
        let trend = detect_monotonic_trend(&values);
        assert!(trend.is_some());
        assert!(trend.unwrap() > 0.0);
    }

    #[test]
    fn test_detect_monotonic_trend_flat() {
        let values = vec![10.0, 10.0, 10.0, 10.0];
        let trend = detect_monotonic_trend(&values);
        assert!(trend.is_none());
    }

    #[test]
    fn test_detect_recurring_failures() {
        let now = Utc::now();
        let observations: Vec<Observation> = (0..5)
            .map(|i| Observation {
                id: format!("obs-{}", i),
                ts: now,
                source: "service".to_string(),
                data: serde_json::json!({
                    "failed": 1,
                    "failed_names": ["nginx.service"],
                }),
            })
            .collect();

        let patterns = detect_recurring_failures(&observations);
        assert!(!patterns.is_empty());
        assert!(patterns[0].name.contains("nginx"));
    }

    #[test]
    fn test_detect_anomalies_no_spike() {
        let now = Utc::now();
        let observations: Vec<Observation> = (0..5)
            .map(|i| Observation {
                id: format!("obs-{}", i),
                ts: now,
                source: "journal".to_string(),
                data: serde_json::json!({
                    "error_count": 2,
                    "errors": [],
                }),
            })
            .collect();

        // All same count = no anomaly
        let patterns = detect_anomalies(&observations);
        assert!(patterns.is_empty());
    }
}
