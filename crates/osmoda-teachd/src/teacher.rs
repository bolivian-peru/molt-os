use anyhow::Result;

use crate::knowledge::{list_knowledge_docs, KnowledgeDoc};

/// Query knowledge docs relevant to a given context string.
/// Returns docs whose title, content, tags, or category match any context keywords.
pub fn teach_context(
    db: &rusqlite::Connection,
    context: &str,
) -> Result<(Vec<KnowledgeDoc>, usize)> {
    // Get all recent knowledge docs
    let all_docs = list_knowledge_docs(db, None, None, 100)?;

    if all_docs.is_empty() {
        return Ok((Vec::new(), 0));
    }

    // Tokenize context into keywords
    let keywords: Vec<String> = context
        .to_lowercase()
        .split_whitespace()
        .filter(|w| w.len() > 2) // Skip very short words
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
        .filter(|w| !w.is_empty())
        .collect();

    if keywords.is_empty() {
        return Ok((Vec::new(), 0));
    }

    // Score each doc by keyword relevance
    let mut scored: Vec<(f64, &KnowledgeDoc)> = all_docs
        .iter()
        .map(|doc| {
            let mut score = 0.0;
            let searchable = format!(
                "{} {} {} {}",
                doc.title.to_lowercase(),
                doc.content.to_lowercase(),
                doc.category.to_lowercase(),
                doc.tags.join(" ").to_lowercase(),
            );

            for kw in &keywords {
                if searchable.contains(kw.as_str()) {
                    score += 1.0;
                }
            }

            // Boost by confidence
            score *= doc.confidence;

            (score, doc)
        })
        .filter(|(score, _)| *score > 0.0)
        .collect();

    // Sort by score descending
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Take top results, respecting a ~1500 token budget (~6000 chars)
    let mut injected_chars = 0usize;
    let max_chars = 6000;
    let mut relevant_docs = Vec::new();

    for (_, doc) in scored {
        let doc_chars = doc.content.len() + doc.title.len() + 20; // rough overhead
        if injected_chars + doc_chars > max_chars && !relevant_docs.is_empty() {
            break;
        }
        injected_chars += doc_chars;
        relevant_docs.push(doc.clone());
    }

    // Approximate token count (1 token ≈ 4 chars)
    let injected_tokens = injected_chars / 4;

    Ok((relevant_docs, injected_tokens))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::{init_db, insert_knowledge_doc, KnowledgeDoc};
    use chrono::Utc;
    use rusqlite::Connection;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        conn
    }

    #[test]
    fn test_teach_context_finds_relevant_docs() {
        let conn = setup_db();
        let now = Utc::now();

        insert_knowledge_doc(
            &conn,
            &KnowledgeDoc {
                id: "kd-1".to_string(),
                title: "nginx memory leak detected".to_string(),
                category: "performance".to_string(),
                content: "nginx is leaking memory over time".to_string(),
                source_patterns: vec![],
                confidence: 0.9,
                created_at: now,
                updated_at: now,
                applied: false,
                tags: vec!["nginx".to_string(), "memory".to_string()],
            },
        )
        .unwrap();

        insert_knowledge_doc(
            &conn,
            &KnowledgeDoc {
                id: "kd-2".to_string(),
                title: "disk filling at 2GB/day".to_string(),
                category: "reliability".to_string(),
                content: "Disk usage is increasing steadily".to_string(),
                source_patterns: vec![],
                confidence: 0.8,
                created_at: now,
                updated_at: now,
                applied: false,
                tags: vec!["disk".to_string()],
            },
        )
        .unwrap();

        let (docs, tokens) = teach_context(&conn, "nginx is using too much memory").unwrap();
        assert!(!docs.is_empty());
        assert!(tokens > 0);
        assert_eq!(docs[0].id, "kd-1");
    }

    #[test]
    fn test_teach_context_empty_when_no_match() {
        let conn = setup_db();
        let (docs, tokens) = teach_context(&conn, "unrelated query about databases").unwrap();
        assert!(docs.is_empty());
        assert_eq!(tokens, 0);
    }
}
