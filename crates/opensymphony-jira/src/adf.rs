//! Plain-text rendering of Atlassian Document Format (ADF) payloads.
//!
//! Jira Cloud REST API v3 returns rich-text fields (issue descriptions and
//! comment bodies) as ADF documents. The orchestrator consumes tracker text as
//! markdown-ish plain text, so this module flattens ADF into text while
//! preserving the structural cues the rest of the system keys on (notably
//! `## `-style headings for workpad markers).

use serde_json::Value;

/// Renders a Jira rich-text field to plain text. Accepts either a plain string
/// (Jira Data Center / API v2) or an ADF document object (Jira Cloud API v3).
/// Returns `None` when the field is absent or renders to an empty string.
pub(crate) fn document_text(value: &Value) -> Option<String> {
    let text = match value {
        Value::Null => return None,
        Value::String(text) => text.clone(),
        Value::Object(_) => {
            let mut blocks = Vec::new();
            render_content(value.get("content"), &mut blocks, 0);
            blocks.join("\n")
        }
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn render_content(content: Option<&Value>, blocks: &mut Vec<String>, list_depth: usize) {
    let Some(Value::Array(nodes)) = content else {
        return;
    };
    for node in nodes {
        render_node(node, blocks, list_depth);
    }
}

fn render_node(node: &Value, blocks: &mut Vec<String>, list_depth: usize) {
    let node_type = node.get("type").and_then(Value::as_str).unwrap_or_default();
    match node_type {
        "paragraph" => {
            let text = inline_text(node);
            if !text.is_empty() {
                blocks.push(text);
            }
        }
        "heading" => {
            let level = node
                .get("attrs")
                .and_then(|attrs| attrs.get("level"))
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 6) as usize;
            blocks.push(format!("{} {}", "#".repeat(level), inline_text(node)));
        }
        "bulletList" | "orderedList" => {
            render_content(node.get("content"), blocks, list_depth + 1);
        }
        "listItem" => {
            let mut item_blocks = Vec::new();
            render_content(node.get("content"), &mut item_blocks, list_depth);
            let indent = "  ".repeat(list_depth.saturating_sub(1));
            for (index, block) in item_blocks.into_iter().enumerate() {
                if index == 0 {
                    blocks.push(format!("{indent}- {block}"));
                } else {
                    blocks.push(format!("{indent}  {block}"));
                }
            }
        }
        "codeBlock" => {
            blocks.push(format!("```\n{}\n```", inline_text(node)));
        }
        "blockquote" => {
            let mut quoted = Vec::new();
            render_content(node.get("content"), &mut quoted, list_depth);
            for block in quoted {
                blocks.push(format!("> {block}"));
            }
        }
        "rule" => blocks.push("---".to_string()),
        // Panels, tables, media groups, expands, etc. — flatten their children.
        _ => render_content(node.get("content"), blocks, list_depth),
    }
}

fn inline_text(node: &Value) -> String {
    let mut out = String::new();
    collect_inline_text(node.get("content"), &mut out);
    out.trim_end().to_string()
}

fn collect_inline_text(content: Option<&Value>, out: &mut String) {
    let Some(Value::Array(nodes)) = content else {
        return;
    };
    for node in nodes {
        let node_type = node.get("type").and_then(Value::as_str).unwrap_or_default();
        match node_type {
            "text" => {
                if let Some(text) = node.get("text").and_then(Value::as_str) {
                    out.push_str(text);
                }
            }
            "hardBreak" => out.push('\n'),
            "mention" | "emoji" | "status" | "date" | "placeholder" => {
                if let Some(text) = node
                    .get("attrs")
                    .and_then(|attrs| attrs.get("text").or_else(|| attrs.get("shortName")))
                    .and_then(Value::as_str)
                {
                    out.push_str(text);
                }
            }
            "inlineCard" => {
                if let Some(url) = node
                    .get("attrs")
                    .and_then(|attrs| attrs.get("url"))
                    .and_then(Value::as_str)
                {
                    out.push_str(url);
                }
            }
            _ => collect_inline_text(node.get("content"), out),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::document_text;

    #[test]
    fn plain_strings_pass_through() {
        assert_eq!(
            document_text(&json!("already plain")).as_deref(),
            Some("already plain")
        );
        assert_eq!(document_text(&json!("   ")), None);
        assert_eq!(document_text(&json!(null)), None);
    }

    #[test]
    fn adf_headings_render_with_markdown_markers() {
        let doc = json!({
            "type": "doc",
            "version": 1,
            "content": [
                {
                    "type": "heading",
                    "attrs": {"level": 2},
                    "content": [{"type": "text", "text": "Agent Harness Workpad"}]
                },
                {
                    "type": "paragraph",
                    "content": [{"type": "text", "text": "Keep going."}]
                }
            ]
        });

        assert_eq!(
            document_text(&doc).as_deref(),
            Some("## Agent Harness Workpad\nKeep going.")
        );
    }

    #[test]
    fn adf_lists_code_blocks_and_mentions_flatten_to_text() {
        let doc = json!({
            "type": "doc",
            "version": 1,
            "content": [
                {
                    "type": "bulletList",
                    "content": [
                        {
                            "type": "listItem",
                            "content": [{
                                "type": "paragraph",
                                "content": [{"type": "text", "text": "first"}]
                            }]
                        },
                        {
                            "type": "listItem",
                            "content": [{
                                "type": "paragraph",
                                "content": [
                                    {"type": "mention", "attrs": {"id": "1", "text": "@sam"}},
                                    {"type": "text", "text": " second"}
                                ]
                            }]
                        }
                    ]
                },
                {
                    "type": "codeBlock",
                    "content": [{"type": "text", "text": "let x = 1;"}]
                }
            ]
        });

        assert_eq!(
            document_text(&doc).as_deref(),
            Some("- first\n- @sam second\n```\nlet x = 1;\n```")
        );
    }
}
