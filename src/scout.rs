// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// scout.rs — The Scout Agent (Phase 4)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// This module bridges the mHTTP data ingestion engine with the local LLM
// to autonomously evaluate a target developer's Web3/Rust compatibility.
// It directly returns structured JSON assessments without executing code.

use crate::mhttp;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const OLLAMA_ENDPOINT: &str = "http://127.0.0.1:11434/api/generate";
const MODEL_NAME: &str = "qwen2.5-coder:latest";

#[derive(Serialize)]
struct GenerateRequest {
    model: String,
    prompt: String,
    stream: bool,
    format: String,
}

#[derive(Deserialize)]
struct GenerateResponse {
    response: String,
}

#[derive(Deserialize, Debug)]
pub struct CandidateEvaluation {
    pub developer_score: u8,
    pub summary_rationale: String,
    pub technical_strengths: Vec<String>,
}

/// Evaluates a candidate based on the provided URL (e.g. a GitHub profile/README).
/// Ingests via mHTTP, evaluates via Ollama, and returns structured JSON telemetry.
pub async fn evaluate_candidate(target_url: &str) -> Result<CandidateEvaluation, String> {
    println!("  [SCOUT] Ingesting target identity: {}", target_url);

    // 1. Fetch and compress data via mHTTP
    let raw_payload = mhttp::fetch_and_compress(target_url)
        .await
        .map_err(|e| format!("mHTTP ingestion failed: {}", e))?;

    println!("  [SCOUT] Target ingested successfully ({} bytes). Evaluating...", raw_payload.len());

    // 2. Prepare LLM Request for JSON response
    let system_prompt = format!(
        "You are an elite Senior Technical Recruiter specializing in Web3, Rust, and Solana.\n\
        Evaluate the following text scraped from a developer's repository or profile.\n\
        You must return a STRICT JSON object representing your assessment.\n\
        Do NOT wrap the JSON in markdown blocks (no ```json). Reply ONLY with the raw {{...}} JSON.\n\
        \n\
        The JSON must strictly match this exact structure:\n\
        {{\n\
        \"developer_score\": <number out of 10>,\n\
        \"summary_rationale\": \"<a 2-3 sentence strict technical judgment>\",\n\
        \"technical_strengths\": [\"<strength 1>\", \"<strength 2>\"]\n\
        }}\n\n\
        Here is the payload to evaluate:\n\
        ---\n\
        {}\n\
        ---\n",
        raw_payload
    );

    let req_body = GenerateRequest {
        model: MODEL_NAME.to_string(),
        prompt: system_prompt,
        stream: false,
        format: "json".to_string(),
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let res = client
        .post(OLLAMA_ENDPOINT)
        .json(&req_body)
        .send()
        .await
        .map_err(|e| format!("Ollama API request failed: {}", e))?;

    if !res.status().is_success() {
        return Err(format!("Ollama returned HTTP error: {}", res.status()));
    }

    let gen_resp: GenerateResponse = res
        .json()
        .await
        .map_err(|e| format!("Failed to parse LLM response metadata: {}", e))?;

    // 3. Parse JSON Evaluation
    let clean_json = gen_resp.response.trim();

    let evaluation: CandidateEvaluation = serde_json::from_str(clean_json)
        .map_err(|e| format!("LLM did not return valid JSON structure. Error: {}. Raw Response: {}", e, clean_json))?;

    Ok(evaluation)
}
