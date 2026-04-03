use crate::error::NarratorError;
use crate::models::ProcessedDocument;
use std::path::Path;

pub fn process_document(path: &Path) -> Result<ProcessedDocument, NarratorError> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let content = match extension.as_str() {
        "md" | "markdown" => process_markdown(path)?,
        "txt" | "text" => process_text(path)?,
        "pdf" => process_pdf(path)?,
        other => {
            return Err(NarratorError::DocumentError(format!(
                "Unsupported document type: .{other}"
            )));
        }
    };

    let token_estimate = estimate_tokens(&content);

    Ok(ProcessedDocument {
        name,
        content,
        token_estimate,
        source_path: path.to_string_lossy().to_string(),
    })
}

fn process_markdown(path: &Path) -> Result<String, NarratorError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| NarratorError::DocumentError(format!("Failed to read {}: {e}", path.display())))?;
    // Return raw markdown — Claude handles markdown well
    Ok(raw)
}

fn process_text(path: &Path) -> Result<String, NarratorError> {
    std::fs::read_to_string(path)
        .map_err(|e| NarratorError::DocumentError(format!("Failed to read {}: {e}", path.display())))
}

fn process_pdf(path: &Path) -> Result<String, NarratorError> {
    // Basic PDF text extraction — read the file bytes and try to extract text
    // Using a simple approach: if pdf-extract is available, use it
    // For now, return an error suggesting the user convert to text
    let bytes = std::fs::read(path)
        .map_err(|e| NarratorError::DocumentError(format!("Failed to read PDF {}: {e}", path.display())))?;

    // Try basic text extraction from PDF bytes
    // Look for text between BT and ET markers (simplified PDF text extraction)
    let content = extract_pdf_text_basic(&bytes);
    if content.trim().is_empty() {
        return Err(NarratorError::DocumentError(
            "Could not extract text from PDF. Try converting to .txt or .md first.".to_string(),
        ));
    }
    Ok(content)
}

fn extract_pdf_text_basic(bytes: &[u8]) -> String {
    // Very basic PDF text extraction — looks for readable ASCII sequences
    // This is a fallback; a proper implementation would use a PDF library
    let text = String::from_utf8_lossy(bytes);
    let mut result = String::new();
    let mut in_text = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.contains("BT") {
            in_text = true;
            continue;
        }
        if trimmed.contains("ET") {
            in_text = false;
            continue;
        }
        if in_text {
            // Extract text from Tj or TJ operators
            if let Some(start) = trimmed.find('(') {
                if let Some(end) = trimmed.rfind(')') {
                    if start < end {
                        result.push_str(&trimmed[start + 1..end]);
                        result.push(' ');
                    }
                }
            }
        }
    }

    // If BT/ET extraction fails, just grab printable ASCII
    if result.trim().is_empty() {
        for &byte in bytes {
            if byte >= 32 && byte < 127 {
                result.push(byte as char);
            } else if byte == b'\n' || byte == b'\r' {
                result.push('\n');
            }
        }
    }

    result
}

pub fn estimate_tokens(text: &str) -> usize {
    // Rough approximation: ~4 characters per token for English
    text.len() / 4
}

pub fn truncate_to_budget(
    mut docs: Vec<ProcessedDocument>,
    max_tokens: usize,
) -> Vec<ProcessedDocument> {
    let total: usize = docs.iter().map(|d| d.token_estimate).sum();
    if total <= max_tokens {
        return docs;
    }

    // Sort by priority: shorter docs first (glossaries, guides) before long docs
    docs.sort_by_key(|d| d.token_estimate);

    let mut budget_remaining = max_tokens;
    let mut result = Vec::new();

    for mut doc in docs {
        if doc.token_estimate <= budget_remaining {
            budget_remaining -= doc.token_estimate;
            result.push(doc);
        } else if budget_remaining > 100 {
            // Truncate this document to fit
            let char_budget = budget_remaining * 4;
            if doc.content.len() > char_budget {
                doc.content = doc.content[..char_budget].to_string();
                doc.content.push_str("\n\n[... document truncated to fit token budget ...]");
                doc.token_estimate = budget_remaining;
            }
            result.push(doc);
            break;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens("hello world"), 2); // 11 chars / 4 = 2
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 0); // 1/4 = 0
        assert_eq!(estimate_tokens("abcdefgh"), 2); // 8/4 = 2
    }

    #[test]
    fn test_truncate_to_budget_within_budget() {
        let docs = vec![
            ProcessedDocument {
                name: "doc1.txt".to_string(),
                content: "hello".to_string(),
                token_estimate: 100,
                source_path: "/tmp/doc1.txt".to_string(),
            },
            ProcessedDocument {
                name: "doc2.txt".to_string(),
                content: "world".to_string(),
                token_estimate: 200,
                source_path: "/tmp/doc2.txt".to_string(),
            },
        ];

        let result = truncate_to_budget(docs, 500);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_truncate_to_budget_exceeds() {
        let docs = vec![
            ProcessedDocument {
                name: "big.txt".to_string(),
                content: "x".repeat(10000),
                token_estimate: 5000,
                source_path: "/tmp/big.txt".to_string(),
            },
            ProcessedDocument {
                name: "small.txt".to_string(),
                content: "hello".to_string(),
                token_estimate: 100,
                source_path: "/tmp/small.txt".to_string(),
            },
        ];

        let result = truncate_to_budget(docs, 500);
        let total: usize = result.iter().map(|d| d.token_estimate).sum();
        assert!(total <= 500);
    }

    #[test]
    fn test_process_text_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "Hello, this is a test document.").unwrap();

        let result = process_document(&file_path).unwrap();
        assert_eq!(result.name, "test.txt");
        assert!(result.content.contains("Hello"));
        assert!(result.token_estimate > 0);
    }

    #[test]
    fn test_process_markdown_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.md");
        std::fs::write(&file_path, "# Title\n\nSome **bold** text.").unwrap();

        let result = process_document(&file_path).unwrap();
        assert_eq!(result.name, "test.md");
        assert!(result.content.contains("# Title"));
    }

    #[test]
    fn test_unsupported_format() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.xyz");
        std::fs::write(&file_path, "data").unwrap();

        let result = process_document(&file_path);
        assert!(result.is_err());
    }
}
