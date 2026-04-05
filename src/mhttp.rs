// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// mhttp.rs — Machine Hypertext Transfer Protocol
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Host-side data ingestion engine for the Guardrail AI swarm. This module
// runs NATIVELY on the host (Apple Silicon M1), NOT inside the WASM sandbox.
//
// Pipeline:
//   1. Fetch raw HTML payload from target URL via reqwest (async, TLS)
//   2. Strip ALL DOM bloat: HTML tags, CSS, JavaScript, nav, footers,
//      hidden elements, inline styles — everything that isn't signal
//   3. Collapse extraneous whitespace into mathematically dense text
//   4. Truncate to MAX_OUTPUT_CHARS to prevent token budget blowout
//   5. Return pure-signal text ready for LLM consumption
//
// SECURITY NOTE: This module has FULL network access because it runs on
// the host. The WASM sandbox agents NEVER get network access — they receive
// pre-fetched, sanitized text through the host pipeline.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use std::time::Duration;

// ── Constants ────────────────────────────────────────────────────────────────

/// Maximum characters in the final output. Any content beyond this is
/// surgically truncated to prevent token budget overruns in the LLM pipeline.
/// 50,000 chars ≈ ~12,500 tokens (at ~4 chars/token average).
const MAX_OUTPUT_CHARS: usize = 50_000;

/// HTTP request timeout. We fail hard rather than hang indefinitely.
const REQUEST_TIMEOUT_SECS: u64 = 15;

/// Maximum raw HTML payload size we'll accept (10 MB).
/// Anything larger is almost certainly not a document we want to ingest.
const MAX_RAW_PAYLOAD_BYTES: usize = 10 * 1024 * 1024;

/// Rendering width for html2text. Controls line wrapping in the output.
/// 120 chars is wide enough for dense information without absurd line lengths.
const RENDER_WIDTH: usize = 120;

// ── Public API ───────────────────────────────────────────────────────────────

/// Fetch a URL, annihilate the DOM, and return token-optimized text.
///
/// This is the primary entry point for the mHTTP data ingestion pipeline.
///
/// # Pipeline Stages
/// 1. **Fetch**: Async HTTP GET with 15s timeout, follow redirects, TLS
/// 2. **Size Guard**: Reject payloads > 10MB before parsing
/// 3. **DOM Strip**: html2text nukes all HTML/CSS/JS into plain text
/// 4. **Whitespace Collapse**: Regex-free multi-pass normalization
/// 5. **Truncation**: Hard cap at 50,000 characters
///
/// # Returns
/// - `Ok(String)` — Clean, dense text. Pure signal.
/// - `Err(String)` — Human-readable error describing the failure mode.
///
/// # Error Modes (all gracefully handled)
/// - Network timeout (15s)
/// - DNS resolution failure
/// - HTTP 4xx/5xx status codes
/// - Payload too large (>10MB)
/// - Invalid/unparseable HTML (degrades gracefully — html2text is robust)
pub async fn fetch_and_compress(url: &str) -> Result<String, String> {
    // ── Stage 1: Fetch raw payload ───────────────────────────────────────
    let raw_html = fetch_raw(url).await?;
    let raw_size = raw_html.len();

    // ── Stage 2: DOM annihilation ────────────────────────────────────────
    // html2text takes the raw HTML byte stream and renders it as plain text,
    // stripping all tags, scripts, styles, and structural markup.
    let text = html2text::config::with_decorator(html2text::render::PlainDecorator::new())
        .string_from_read(raw_html.as_bytes(), RENDER_WIDTH)
        .map_err(|e| format!("DOM parsing failed: {}", e))?;

    // ── Stage 3: Whitespace collapse ─────────────────────────────────────
    // The raw text output often has excessive blank lines from stripped
    // elements. We collapse these into dense, token-efficient text.
    let compressed = collapse_whitespace(&text);

    // ── Stage 4: Truncation guard ────────────────────────────────────────
    let final_text = if compressed.len() > MAX_OUTPUT_CHARS {
        let mut truncated = compressed[..MAX_OUTPUT_CHARS].to_string();
        truncated.push_str("\n\n[TRUNCATED — exceeded 50,000 character limit]");
        truncated
    } else {
        compressed
    };

    let compressed_size = final_text.len();
    let ratio = if raw_size > 0 {
        ((1.0 - (compressed_size as f64 / raw_size as f64)) * 100.0) as u32
    } else {
        0
    };

    // Prepend compression metadata for the LLM telemetry loop.
    let output = format!(
        "[mHTTP] Source: {}\n\
         [mHTTP] Raw HTML: {} bytes → Compressed: {} bytes ({}% reduction)\n\
         [mHTTP] ─────────────────────────────────────────────────────────\n\n\
         {}",
        url, raw_size, compressed_size, ratio, final_text
    );

    Ok(output)
}

/// Fetch raw payload — returns the complete body and the raw byte count.
/// Returned separately so main.rs can report compression ratios.
pub async fn fetch_raw_payload(url: &str) -> Result<(String, usize), String> {
    let raw_html = fetch_raw(url).await?;
    let raw_size = raw_html.len();
    Ok((raw_html, raw_size))
}

/// Strip and compress HTML — takes raw HTML string, returns dense text.
/// Separated from fetch so main.rs can measure each stage independently.
pub fn compress_html(raw_html: &str) -> Result<(String, usize), String> {
    let text = html2text::config::with_decorator(html2text::render::PlainDecorator::new())
        .string_from_read(raw_html.as_bytes(), RENDER_WIDTH)
        .map_err(|e| format!("DOM parsing failed: {}", e))?;

    let compressed = collapse_whitespace(&text);

    let final_text = if compressed.len() > MAX_OUTPUT_CHARS {
        let mut truncated = compressed[..MAX_OUTPUT_CHARS].to_string();
        truncated.push_str("\n\n[TRUNCATED — exceeded 50,000 character limit]");
        truncated
    } else {
        compressed
    };

    let size = final_text.len();
    Ok((final_text, size))
}

// ── Internal Implementation ──────────────────────────────────────────────────

/// Fetch raw HTML from a URL with timeout, redirect following, and size guard.
async fn fetch_raw(url: &str) -> Result<String, String> {
    // Build the HTTP client with strict timeout and redirect policy.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("Guardrail-mHTTP/1.0 (AI-Swarm-Data-Ingestion)")
        .build()
        .map_err(|e| format!("HTTP client initialization failed: {}", e))?;

    // Execute the GET request.
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!("Request timed out after {}s: {}", REQUEST_TIMEOUT_SECS, e)
            } else if e.is_connect() {
                format!("Connection failed (DNS/network): {}", e)
            } else {
                format!("HTTP request failed: {}", e)
            }
        })?;

    // Check HTTP status — reject 4xx/5xx.
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "HTTP {} — server rejected request: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("Unknown")
        ));
    }

    // Check content-length header if available (pre-download guard).
    if let Some(content_length) = response.content_length() {
        if content_length as usize > MAX_RAW_PAYLOAD_BYTES {
            return Err(format!(
                "Payload too large: {} bytes (limit: {} bytes)",
                content_length, MAX_RAW_PAYLOAD_BYTES
            ));
        }
    }

    // Download the body.
    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    // Post-download size guard (Content-Length can lie or be absent).
    if body.len() > MAX_RAW_PAYLOAD_BYTES {
        return Err(format!(
            "Downloaded payload too large: {} bytes (limit: {} bytes)",
            body.len(),
            MAX_RAW_PAYLOAD_BYTES
        ));
    }

    Ok(body)
}

/// Collapse extraneous whitespace into dense, token-efficient text.
///
/// This is a regex-free, zero-allocation-per-char approach:
/// 1. Trim leading/trailing whitespace from every line
/// 2. Collapse runs of 3+ blank lines into exactly 2
/// 3. Strip trailing whitespace from the final output
fn collapse_whitespace(input: &str) -> String {
    let mut result = String::with_capacity(input.len() / 2);
    let mut consecutive_blank_lines = 0;

    for line in input.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            consecutive_blank_lines += 1;
            // Allow at most 1 blank line between content blocks.
            if consecutive_blank_lines <= 1 {
                result.push('\n');
            }
        } else {
            consecutive_blank_lines = 0;
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(trimmed);
            result.push('\n');
        }
    }

    // Strip trailing whitespace/newlines.
    let trimmed = result.trim_end().to_string();
    trimmed
}

// ── Unit Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collapse_whitespace_basic() {
        let input = "Hello\n\n\n\n\nWorld\n\n\nFoo";
        let result = collapse_whitespace(input);
        assert_eq!(result, "Hello\n\nWorld\n\nFoo");
    }

    #[test]
    fn test_collapse_whitespace_preserves_single_blank() {
        let input = "Line 1\n\nLine 2";
        let result = collapse_whitespace(input);
        assert_eq!(result, "Line 1\n\nLine 2");
    }

    #[test]
    fn test_collapse_whitespace_strips_trailing() {
        let input = "Content\n\n\n\n";
        let result = collapse_whitespace(input);
        assert_eq!(result, "Content");
    }

    #[test]
    fn test_truncation() {
        let long_input = "x".repeat(60_000);
        let (result, _) = compress_html(&long_input).unwrap();
        assert!(result.len() <= MAX_OUTPUT_CHARS + 100); // +100 for truncation message
    }
}
