use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A research brief that captures targeted findings for a planning session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchBrief {
    pub topic: String,
    pub research_context: String,
    pub findings: Vec<ResearchFinding>,
    pub generated_at: DateTime<Utc>,
}

/// A single research finding with citation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchFinding {
    pub summary: String,
    pub source_url: Option<String>,
    pub source_title: Option<String>,
    pub confidence: ConfidenceLevel,
    pub tags: Vec<String>,
}

/// Confidence in a research finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceLevel {
    High,
    Medium,
    Low,
}

/// Error type for research operations.
#[derive(Debug, thiserror::Error)]
pub enum ResearchError {
    #[error("missing required field: {0}")]
    MissingField(String),
}

/// Builder for constructing a ResearchBrief.
pub struct ResearchBriefBuilder {
    topic: String,
    research_context: String,
    findings: Vec<ResearchFinding>,
}

impl ResearchBriefBuilder {
    pub fn new(topic: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            research_context: String::new(),
            findings: Vec::new(),
        }
    }

    pub fn context(mut self, ctx: impl Into<String>) -> Self {
        self.research_context = ctx.into();
        self
    }

    pub fn add_finding(
        mut self,
        summary: impl Into<String>,
        source_url: Option<&str>,
        confidence: ConfidenceLevel,
    ) -> Self {
        let source_url = source_url.map(|u| u.to_string());
        self.findings.push(ResearchFinding {
            summary: summary.into(),
            source_url,
            source_title: None,
            confidence,
            tags: Vec::new(),
        });
        self
    }

    pub fn add_finding_with_title(
        mut self,
        summary: impl Into<String>,
        source_url: Option<&str>,
        source_title: impl Into<String>,
        confidence: ConfidenceLevel,
        tags: Vec<String>,
    ) -> Self {
        let source_url = source_url.map(|u| u.to_string());
        self.findings.push(ResearchFinding {
            summary: summary.into(),
            source_url,
            source_title: Some(source_title.into()),
            confidence,
            tags,
        });
        self
    }

    pub fn build(self) -> Result<ResearchBrief, ResearchError> {
        if self.topic.is_empty() {
            return Err(ResearchError::MissingField("topic".to_string()));
        }
        Ok(ResearchBrief {
            topic: self.topic,
            research_context: self.research_context,
            findings: self.findings,
            generated_at: Utc::now(),
        })
    }
}

impl ResearchBrief {
    /// Returns findings filtered by confidence level.
    pub fn findings_above(&self, min_confidence: ConfidenceLevel) -> Vec<&ResearchFinding> {
        self.findings
            .iter()
            .filter(|f| {
                Self::confidence_rank(f.confidence) >= Self::confidence_rank(min_confidence)
            })
            .collect()
    }

    /// Returns a markdown projection suitable for review.
    pub fn render_markdown(&self) -> String {
        let mut md = format!("# Research Brief: {}\n\n", self.topic);

        if !self.research_context.is_empty() {
            md.push_str(&format!("**Context:** {}\n\n", self.research_context));
        }

        md.push_str(&format!(
            "**Generated:** {}\n\n",
            self.generated_at.format("%Y-%m-%d %H:%M:%SZ")
        ));
        md.push_str("## Findings\n\n");

        for (i, finding) in self.findings.iter().enumerate() {
            md.push_str(&format!(
                "### {} {}\n\n{}\n",
                i + 1,
                Self::confidence_label(finding.confidence),
                finding.summary
            ));

            if let Some(ref url) = finding.source_url {
                let title = finding.source_title.as_deref().unwrap_or(url.as_str());
                md.push_str(&format!("- **Source:** [{}]({})\n", title, url));
            }

            if !finding.tags.is_empty() {
                md.push_str(&format!("- **Tags:** {}\n", finding.tags.join(", ")));
            }
            md.push('\n');
        }

        md
    }

    fn confidence_rank(level: ConfidenceLevel) -> u8 {
        match level {
            ConfidenceLevel::High => 3,
            ConfidenceLevel::Medium => 2,
            ConfidenceLevel::Low => 1,
        }
    }

    fn confidence_label(level: ConfidenceLevel) -> &'static str {
        match level {
            ConfidenceLevel::High => "🟢 High",
            ConfidenceLevel::Medium => "🟡 Medium",
            ConfidenceLevel::Low => "🔴 Low",
        }
    }
}

/// Stores research artifacts keyed by topic for a planning session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchArtifactStore {
    pub briefs: BTreeMap<String, ResearchBrief>,
}

impl ResearchArtifactStore {
    pub fn new() -> Self {
        Self {
            briefs: BTreeMap::new(),
        }
    }

    pub fn insert(&mut self, brief: ResearchBrief) {
        self.briefs.insert(brief.topic.clone(), brief);
    }

    pub fn get(&self, topic: &str) -> Option<&ResearchBrief> {
        self.briefs.get(topic)
    }

    pub fn topics(&self) -> Vec<String> {
        self.briefs.keys().cloned().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.briefs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.briefs.len()
    }

    /// Renders all briefs as a single markdown document.
    pub fn render_all_markdown(&self) -> String {
        let mut md = String::from("# Research Artifacts\n\n");
        for (i, (_topic, brief)) in self.briefs.iter().enumerate() {
            if i > 0 {
                md.push_str("\n---\n\n");
            }
            md.push_str(&brief.render_markdown());
        }
        md
    }
}

impl Default for ResearchArtifactStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn builder_constructs_valid_brief() {
        let brief = ResearchBriefBuilder::new("Test Topic")
            .context("Researching test integrations")
            .add_finding(
                "Found a relevant API",
                Some("https://example.com/api"),
                ConfidenceLevel::High,
            )
            .build()
            .expect("should build successfully");

        assert_eq!(brief.topic, "Test Topic");
        assert_eq!(brief.findings.len(), 1);
        assert_eq!(brief.findings[0].confidence, ConfidenceLevel::High);
        assert!(brief.findings[0].source_url.is_some());
    }

    #[test]
    fn builder_rejects_empty_topic() {
        let result = ResearchBriefBuilder::new("").build();
        assert!(result.is_err());
        match result.unwrap_err() {
            ResearchError::MissingField(field) => {
                assert_eq!(field, "topic");
            }
        }
    }

    #[test]
    fn findings_above_filters_by_confidence() {
        let brief = ResearchBriefBuilder::new("Test")
            .add_finding("High conf", None, ConfidenceLevel::High)
            .add_finding("Medium conf", None, ConfidenceLevel::Medium)
            .add_finding("Low conf", None, ConfidenceLevel::Low)
            .build()
            .unwrap();

        let high_and_above = brief.findings_above(ConfidenceLevel::High);
        assert_eq!(high_and_above.len(), 1);

        let medium_and_above = brief.findings_above(ConfidenceLevel::Medium);
        assert_eq!(medium_and_above.len(), 2);

        let all = brief.findings_above(ConfidenceLevel::Low);
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn render_markdown_includes_all_findings() {
        let brief = ResearchBriefBuilder::new("API Research")
            .context("Evaluating integration options")
            .add_finding_with_title(
                "OpenHands supports agent-server protocol",
                Some("https://docs.all-hands.dev"),
                "OpenHands Docs",
                ConfidenceLevel::High,
                vec!["api".to_string(), "protocol".to_string()],
            )
            .build()
            .unwrap();

        let md = brief.render_markdown();
        assert!(md.contains("API Research"));
        assert!(md.contains("Evaluating integration options"));
        assert!(md.contains("OpenHands supports agent-server protocol"));
        assert!(md.contains("OpenHands Docs"));
        assert!(md.contains("api"));
    }

    #[test]
    fn research_store_insert_and_retrieve() {
        let mut store = ResearchArtifactStore::new();

        let brief = ResearchBriefBuilder::new("Topic A")
            .add_finding("Finding A", None, ConfidenceLevel::High)
            .build()
            .unwrap();
        store.insert(brief);

        let brief2 = ResearchBriefBuilder::new("Topic B")
            .add_finding("Finding B", None, ConfidenceLevel::Medium)
            .build()
            .unwrap();
        store.insert(brief2);

        assert_eq!(store.len(), 2);
        assert!(store.get("Topic A").is_some());
        assert!(store.get("Topic B").is_some());
        assert!(store.get("Topic C").is_none());
        assert_eq!(store.topics().len(), 2);
    }

    #[test]
    fn research_store_render_all_markdown() {
        let mut store = ResearchArtifactStore::new();
        store.insert(
            ResearchBriefBuilder::new("Topic X")
                .add_finding("X finding", None, ConfidenceLevel::High)
                .build()
                .unwrap(),
        );
        store.insert(
            ResearchBriefBuilder::new("Topic Y")
                .add_finding("Y finding", None, ConfidenceLevel::Medium)
                .build()
                .unwrap(),
        );

        let md = store.render_all_markdown();
        assert!(md.contains("Research Artifacts"));
        assert!(md.contains("Topic X"));
        assert!(md.contains("Topic Y"));
    }

    #[test]
    fn brief_serializes_to_json() {
        let brief = ResearchBriefBuilder::new("Serialization Test")
            .add_finding(
                "Serde test",
                Some("https://serde.rs"),
                ConfidenceLevel::High,
            )
            .build()
            .unwrap();

        let json = serde_json::to_string(&brief).expect("should serialize");
        assert!(json.contains("Serialization Test"));

        let deserialized: ResearchBrief = serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(deserialized.topic, brief.topic);
        assert_eq!(deserialized.findings.len(), brief.findings.len());
    }
}
