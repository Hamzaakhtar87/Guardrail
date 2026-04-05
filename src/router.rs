// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// router.rs — The Swarm Router (Autonomous Coding Loop)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// This module manages the feedback loop between the local LLM and the WASM
// sandbox. It autonomously prompts, generates, compiles, executes, and
// remediates code until a successful execution is achieved within constraints.

use crate::compiler;
use crate::feedback::GuardrailResult;
use crate::sandbox;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const OLLAMA_ENDPOINT: &str = "http://127.0.0.1:11434/api/generate";
const MODEL_NAME: &str = "qwen2.5-coder:latest"; // Adjust to the user's available coder model if needed.

#[derive(Serialize)]
struct GenerateRequest {
    model: String,
    prompt: String,
    stream: bool,
}

#[derive(Deserialize)]
struct GenerateResponse {
    response: String,
}

/// Run an autonomous coding loop with the given task description.
/// Returns the working Rust code and the final stdout if successful.
pub async fn autonomous_coding_loop(task_description: &str, max_iterations: usize) -> Result<(String, String), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60)) // LLM generation can be slow
        .build()
        .map_err(|e| format!("Failed to build reqwest client: {}", e))?;

    // Initial system prompt + task constraints
    let mut conversation_history = format!(
        "You are an elite, deterministic Rust coder. You write ONLY pure Rust code that runs in a zero-trust WASM WASI Preview 1 sandboxed environment.\n\
        Rules:\n\
        - Return ONLY the Rust code inside a ```rust code block.\n\
        - NO explanations. NO markdown outside the code block.\n\
        - The code MUST have a `fn main()` entrypoint.\n\
        - NO filesystem access, NO network access, NO environment variables.\n\
        - ONLY use the standard library and do not use external crates.\n\
        - You use `println!` to output the final result.\n\
        \n\
        Task:\n{}\n",
        task_description
    );

    for iteration in 1..=max_iterations {
        println!("  [ROUTER] Iteration {}/{} — Prompting LLM...", iteration, max_iterations);

        let req_body = GenerateRequest {
            model: MODEL_NAME.to_string(),
            prompt: conversation_history.clone(),
            stream: false,
        };

        let res = client
            .post(OLLAMA_ENDPOINT)
            .json(&req_body)
            .send()
            .await
            .map_err(|e| format!("LLM request failed: {}", e))?;

        if !res.status().is_success() {
            return Err(format!("Ollama API error: HTTP {}", res.status()));
        }

        let gen_resp: GenerateResponse = res
            .json()
            .await
            .map_err(|e| format!("Failed to parse LLM response: {}", e))?;

        let llm_reply = gen_resp.response.trim();

        // Extract Rust code block
        let source_code = extract_rust_code(llm_reply)
            .ok_or_else(|| "LLM failed to provide a valid ```rust block.".to_string())?;

        println!("  [ROUTER] Code received ({} bytes). Testing in Guardrail sandbox...", source_code.len());

        // We must spawn a blocking thread for synchronous Wasmtime operations
        // to avoid conflicting with the tokio async runtime.
        let source_code_clone = source_code.clone();
        let guardrail_result = tokio::task::spawn_blocking(move || {
            match compiler::compile_to_wasm(&source_code_clone) {
                Ok(artifact) => sandbox::execute_wasm(&artifact.wasm_path),
                Err(e) => GuardrailResult::compilation_error(format!("{:#}", e)),
            }
        })
        .await
        .map_err(|e| format!("Sandbox panic: {}", e))?;

        let json_feedback = guardrail_result.to_json();

        // Check if the code succeeded
        if matches!(guardrail_result, GuardrailResult::Success { .. }) {
            println!("  [ROUTER] SUCCESS! Code executed flawlessly.\n");
            // Pull the success output text out of the json for returning
            // We know it is success so stdout is present.
            let stdout_val = extract_stdout(&json_feedback).unwrap_or_else(|| "<no stdout>".to_string());
            return Ok((source_code, stdout_val));
        }

        // Output error to terminal then inject to history
        println!("  [ROUTER] FAILURE. Execution blocked by Guardrail.\n  {}", summarize_error(&json_feedback));

        if iteration == max_iterations {
            return Err("Max iterations reached. Autonomous loop failed.".to_string());
        }

        // Remediate: Feed error JSON back
        conversation_history.push_str("\n\nYou wrote the following code:\n```rust\n");
        conversation_history.push_str(&source_code);
        conversation_history.push_str("\n```\n\nThe Guardrail sandbox rejected your code with the following telemetry JSON:\n```json\n");
        conversation_history.push_str(&json_feedback);
        conversation_history.push_str("\n```\n\nFix ALL the reported errors and provide the corrected code in a single ```rust block. NO explanations.\n");
    }

    Err("Unreachable".into())
}

/// Extract Rust code bounded by ```rust and ```
fn extract_rust_code(response: &str) -> Option<String> {
    let mut in_block = false;
    let mut code = String::new();

    for line in response.lines() {
        if line.starts_with("```rust") {
            in_block = true;
            continue;
        } else if line.starts_with("```") && in_block {
            return Some(code);
        }

        if in_block {
            code.push_str(line);
            code.push('\n');
        }
    }

    None
}

/// Helper to extract stdout from a GuardrailResult success JSON dump
fn extract_stdout(json_str: &str) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
        if let Some(stdout) = v.get("stdout").and_then(|s| s.as_str()) {
            return Some(stdout.to_string());
        }
    }
    None
}

/// Helper to extract a readable summary from a GuardrailResult JSON dump
fn summarize_error(json_str: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
        if let Some(err_type) = v.get("error_type").and_then(|s| s.as_str()) {
            return format!("Error Type: {}", err_type);
        }
    }
    "Generic Guardrail Error".to_string()
}
