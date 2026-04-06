// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// syndicate.rs — The 10x Coding Syndicate (Phase 6)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// A language-agnostic "Mixture of Agents" (MoA) coding engine that decomposes
// autonomous code generation into three sequential specialist personas:
//
//   Agent 1: THE ARCHITECT
//            Takes the raw task. Outputs a strict implementation plan —
//            data structures, algorithm steps, edge cases. No code.
//
//   Agent 2: THE CODER
//            Takes the Architect's plan. Writes code in the target language.
//
//   Agent 3: THE CRITIC
//            Takes the Coder's output. Hunts for memory leaks, logic errors,
//            unsafe patterns, and inefficiencies. Outputs a refined final
//            version of the code.
//
// GUARDRAIL SANDBOX:
//   If language == "rust", the Critic's output is compiled to .wasm and
//   executed in the Wasmtime sandbox with remediation on failure.
//   For all other languages, the Architect→Coder→Critic loop runs and
//   the final Critic code is returned directly (no sandbox execution).
//   Multi-language runtimes will be added to the sandbox later.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use crate::compiler;
use crate::feedback::GuardrailResult;
use crate::sandbox;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const OLLAMA_ENDPOINT: &str = "http://127.0.0.1:11434/api/generate";
const MODEL_NAME: &str = "qwen2.5-coder:latest";

/// Maximum remediation rounds after the initial Architect→Coder→Critic pass.
/// Each round re-invokes the Coder and Critic with the Guardrail error JSON.
const MAX_REMEDIATION_ROUNDS: usize = 3;

/// HTTP timeout for Ollama requests. 5 minutes to support M1 generation speed
/// on large quantized models (7B+ parameter inference).
const LLM_TIMEOUT_SECS: u64 = 300;

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

// ── Public API ───────────────────────────────────────────────────────────────

/// Run the 10x Coding Syndicate pipeline against a task in any language.
///
/// # Arguments
/// * `task` — The coding task description.
/// * `language` — Target language (e.g. "rust", "python", "go", "javascript").
/// * `expected_output` — Optional expected substring in stdout for logic verification.
///
/// # Behavior
/// - If `language` is "rust": full Guardrail sandbox execution + remediation.
/// - Otherwise: Architect→Coder→Critic only, returns final code directly.
/// - If `expected_output` is Some, stdout must contain the expected string
///   or the code is rejected and sent back for remediation.
pub async fn syndicate_coding_loop(
    task: &str,
    language: &str,
    expected_output: Option<&str>,
) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(LLM_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let is_rust = language.eq_ignore_ascii_case("rust");
    let lang_lower = language.to_lowercase();

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // AGENT 1: THE ARCHITECT
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    println!("  [SYNDICATE] Agent 1/3: THE ARCHITECT — Generating implementation plan...");

    let sandbox_constraints = if is_rust {
        "The code will run in a WASM WASI Preview 1 sandbox with NO filesystem, \
         NO network, NO environment variables, and a 50MB memory limit. \
         ONLY the standard library is available. No external crates. \
         The program must have a `fn main()` entry point and use `println!` for output."
            .to_string()
    } else {
        format!(
            "The code must be a standalone {} program. Use ONLY the standard library. \
             No external packages or dependencies. The program must print its output \
             to stdout.",
            language
        )
    };

    let architect_prompt = format!(
        "You are THE ARCHITECT — an elite systems designer specializing in \
        high-performance {}.\n\n\
        Your job is to produce a STRICT IMPLEMENTATION PLAN for the following task. \
        You must output:\n\
        1. The exact data structures needed (with field types and sizes).\n\
        2. A step-by-step algorithm breakdown (numbered steps).\n\
        3. Edge cases and invariants that MUST be handled.\n\
        4. Memory and performance constraints.\n\n\
        RULES:\n\
        - Do NOT write any {} code. Only the plan.\n\
        - Be extremely specific. Ambiguity kills downstream agents.\n\
        - {}\n\n\
        TASK:\n{}\n",
        language, language, sandbox_constraints, task
    );

    let architecture_plan = prompt_llm(&client, &architect_prompt).await?;
    println!(
        "  [SYNDICATE] Architect delivered plan ({} bytes).",
        architecture_plan.len()
    );

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // AGENT 2: THE CODER
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    println!("  [SYNDICATE] Agent 2/3: THE CODER — Implementing from Architect's plan...");

    let coder_prompt = build_coder_prompt(language, &architecture_plan, None, None);

    let coder_response = prompt_llm(&client, &coder_prompt).await?;
    let initial_code = extract_code(&coder_response, &lang_lower)
        .ok_or_else(|| format!("THE CODER failed to produce a valid ```{} block.", language))?;
    println!(
        "  [SYNDICATE] Coder delivered implementation ({} bytes).",
        initial_code.len()
    );

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // AGENT 3: THE CRITIC
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    println!("  [SYNDICATE] Agent 3/3: THE CRITIC — Reviewing and hardening code...");

    let critic_code = invoke_critic(&client, &initial_code, &architecture_plan, language).await?;
    println!(
        "  [SYNDICATE] Critic delivered hardened code ({} bytes).",
        critic_code.len()
    );

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // GUARDRAIL EXECUTION (Rust only) or DIRECT RETURN
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    if !is_rust {
        // Non-Rust languages: bypass sandbox, return Critic's final code directly.
        println!(
            "  [SYNDICATE] Language '{}' — Guardrail sandbox bypass. Returning Critic's code.",
            language
        );
        return Ok(critic_code);
    }

    // Rust path: full Guardrail sandbox execution + remediation loop.
    let mut current_code = critic_code;

    for round in 0..=MAX_REMEDIATION_ROUNDS {
        let round_label = if round == 0 {
            "Initial submission".to_string()
        } else {
            format!("Remediation {}/{}", round, MAX_REMEDIATION_ROUNDS)
        };
        println!("  [SYNDICATE] Guardrail Execution — {}...", round_label);

        // Compile and execute in the sandbox on a blocking thread
        let code_clone = current_code.clone();
        let guardrail_result = tokio::task::spawn_blocking(move || {
            match compiler::compile_to_wasm(&code_clone) {
                Ok(artifact) => sandbox::execute_wasm(&artifact.wasm_path),
                Err(e) => GuardrailResult::compilation_error(format!("{:#}", e)),
            }
        })
        .await
        .map_err(|e| format!("Sandbox thread panicked: {}", e))?;

        let json_feedback = guardrail_result.to_json();

        // Check for success
        if matches!(guardrail_result, GuardrailResult::Success { .. }) {
            let stdout =
                extract_stdout(&json_feedback).unwrap_or_else(|| "<no stdout>".to_string());

            // ── Phase 7: Deterministic Output Verification ───────────
            if let Some(expected) = expected_output {
                let stdout_trimmed = stdout.trim();
                if stdout_trimmed.contains(expected) {
                    println!(
                        "  [SYNDICATE] ✅ SUCCESS — Output verified on {}.",
                        round_label
                    );
                    return Ok(stdout);
                } else {
                    // Logic failure: sandbox passed but output is wrong.
                    // Construct synthetic error telemetry and force remediation.
                    let logic_error = format!(
                        "[Logic Error] Expected output containing '{}', but got '{}'",
                        expected, stdout_trimmed
                    );
                    println!("  [SYNDICATE] ❌ LOGIC FAILURE — {}", logic_error);

                    if round == MAX_REMEDIATION_ROUNDS {
                        return Err(format!(
                            "Syndicate exhausted all {} remediation rounds. Last error: {}",
                            MAX_REMEDIATION_ROUNDS, logic_error
                        ));
                    }

                    // Feed logic error back to Coder → Critic
                    println!("  [SYNDICATE] Initiating remediation: Coder → Critic with logic error...");

                    let logic_feedback = format!(
                        r#"{{"status": "error", "phase": "verification", "error_type": "logic_error", "message": "{}", "actual_stdout": "{}", "expected_substring": "{}"}}"#,
                        logic_error, stdout_trimmed, expected
                    );

                    let remediation_prompt = build_coder_prompt(
                        language,
                        &architecture_plan,
                        Some(&current_code),
                        Some(&logic_feedback),
                    );

                    let remediated_raw = prompt_llm(&client, &remediation_prompt).await?;
                    let remediated_code = extract_code(&remediated_raw, &lang_lower).ok_or_else(|| {
                        format!(
                            "Remediation Coder failed to produce a valid ```{} block.",
                            language
                        )
                    })?;

                    current_code =
                        invoke_critic(&client, &remediated_code, &architecture_plan, language).await?;
                    println!(
                        "  [SYNDICATE] Remediation Critic delivered hardened code ({} bytes).",
                        current_code.len()
                    );
                    continue;
                }
            } else {
                // No expected output — sandbox success is sufficient.
                println!(
                    "  [SYNDICATE] ✅ SUCCESS — Code passed Guardrail on {}.",
                    round_label
                );
                return Ok(stdout);
            }
        }

        let error_summary = summarize_error(&json_feedback);
        println!("  [SYNDICATE] ❌ REJECTED — {}", error_summary);

        if round == MAX_REMEDIATION_ROUNDS {
            return Err(format!(
                "Syndicate exhausted all {} remediation rounds. Last error: {}",
                MAX_REMEDIATION_ROUNDS, error_summary
            ));
        }

        // ── REMEDIATION: Re-invoke Coder → Critic with error telemetry ───
        println!("  [SYNDICATE] Initiating remediation: Coder → Critic with error JSON...");

        let remediation_prompt = build_coder_prompt(
            language,
            &architecture_plan,
            Some(&current_code),
            Some(&json_feedback),
        );

        let remediated_raw = prompt_llm(&client, &remediation_prompt).await?;
        let remediated_code = extract_code(&remediated_raw, &lang_lower).ok_or_else(|| {
            format!(
                "Remediation Coder failed to produce a valid ```{} block.",
                language
            )
        })?;

        // Run the remediated code through the Critic
        current_code =
            invoke_critic(&client, &remediated_code, &architecture_plan, language).await?;
        println!(
            "  [SYNDICATE] Remediation Critic delivered hardened code ({} bytes).",
            current_code.len()
        );
    }

    Err("Unreachable".into())
}

// ── Internal: Prompt Builders ────────────────────────────────────────────────

/// Build a Coder prompt. If `previous_code` and `error_json` are provided,
/// this becomes a remediation prompt instead of a fresh implementation prompt.
fn build_coder_prompt(
    language: &str,
    architecture_plan: &str,
    previous_code: Option<&str>,
    error_json: Option<&str>,
) -> String {
    let lang_lower = language.to_lowercase();

    let sandbox_rules = if lang_lower == "rust" {
        format!(
            "- The code MUST have a `fn main()` entrypoint.\n\
             - NO filesystem access, NO network access, NO environment variables.\n\
             - ONLY use the standard library. No external crates.\n\
             - Use `println!` to output results.\n\
             - The code runs in a WASM WASI sandbox with a 50MB memory limit."
        )
    } else {
        format!(
            "- Write a standalone {} program.\n\
             - Use ONLY the standard library. No external packages.\n\
             - Print output to stdout.",
            language
        )
    };

    match (previous_code, error_json) {
        (Some(code), Some(error)) => format!(
            "You are THE CODER — an elite {} implementation engineer.\n\n\
            You previously wrote the following code:\n\
            ```{}\n{}\n```\n\n\
            The execution sandbox REJECTED your code with this telemetry:\n\
            ```json\n{}\n```\n\n\
            The original ARCHITECT's plan was:\n\
            ─────────────────────────────────────────────\n\
            {}\n\
            ─────────────────────────────────────────────\n\n\
            Fix ALL reported errors. Return the corrected code in a single ```{} block.\n\
            RULES:\n\
            - NO explanations. ONLY the ```{} block.\n\
            {}\n",
            language, lang_lower, code, error, architecture_plan, lang_lower, lang_lower,
            sandbox_rules
        ),
        _ => format!(
            "You are THE CODER — an elite {} implementation engineer. You write flawless, \
            production-grade {} code from architectural specifications.\n\n\
            Below is the implementation plan from THE ARCHITECT. You must implement it \
            EXACTLY as specified.\n\n\
            ARCHITECT'S PLAN:\n\
            ─────────────────────────────────────────────\n\
            {}\n\
            ─────────────────────────────────────────────\n\n\
            RULES:\n\
            - Return ONLY the {} code inside a ```{} code block.\n\
            - NO explanations. NO markdown outside the code block.\n\
            {}\n",
            language, language, architecture_plan, language, lang_lower, sandbox_rules
        ),
    }
}

// ── Internal: LLM Communication ──────────────────────────────────────────────

/// Send a prompt to the local Ollama endpoint and return the raw response text.
async fn prompt_llm(client: &reqwest::Client, prompt: &str) -> Result<String, String> {
    let req_body = GenerateRequest {
        model: MODEL_NAME.to_string(),
        prompt: prompt.to_string(),
        stream: false,
    };

    let res = client
        .post(OLLAMA_ENDPOINT)
        .json(&req_body)
        .send()
        .await
        .map_err(|e| format!("Ollama request failed: {}", e))?;

    if !res.status().is_success() {
        return Err(format!("Ollama API error: HTTP {}", res.status()));
    }

    let gen_resp: GenerateResponse = res
        .json()
        .await
        .map_err(|e| format!("Failed to parse Ollama response: {}", e))?;

    Ok(gen_resp.response.trim().to_string())
}

// ── Internal: The Critic Agent ───────────────────────────────────────────────

/// Invoke the Critic agent to review and harden code.
/// Returns the refined code extracted from the Critic's response.
async fn invoke_critic(
    client: &reqwest::Client,
    code: &str,
    architecture_plan: &str,
    language: &str,
) -> Result<String, String> {
    let lang_lower = language.to_lowercase();

    let sandbox_check = if lang_lower == "rust" {
        "5. SANDBOX COMPLIANCE: Uses ONLY std, no filesystem/network/env, has fn main()?\n"
            .to_string()
    } else {
        format!(
            "5. STANDALONE: Uses ONLY the {} standard library, no external packages?\n",
            language
        )
    };

    let critic_prompt = format!(
        "You are THE CRITIC — an adversarial code reviewer with zero tolerance for defects.\n\n\
        Below is {} code written by THE CODER based on an architectural plan.\n\
        Your job is to AGGRESSIVELY review this code and output a REFINED, FINAL version.\n\n\
        REVIEW CHECKLIST:\n\
        1. CORRECTNESS: Does the algorithm match the Architect's specification exactly?\n\
        2. MEMORY SAFETY: Any potential panics, overflows, or out-of-bounds access?\n\
        3. LOGIC FLAWS: Off-by-one errors, incorrect bit operations, wrong endianness?\n\
        4. EFFICIENCY: Unnecessary allocations, redundant computations, suboptimal loops?\n\
        {}\
        6. OUTPUT: Does the code print verifiable output to stdout?\n\n\
        ARCHITECT'S PLAN:\n\
        ─────────────────────────────────────────────\n\
        {}\n\
        ─────────────────────────────────────────────\n\n\
        CODER'S IMPLEMENTATION:\n\
        ```{}\n{}\n```\n\n\
        OUTPUT RULES:\n\
        - Return ONLY the refined {} code inside a single ```{} block.\n\
        - If the code is already perfect, return it unchanged inside a ```{} block.\n\
        - NO explanations. NO commentary. ONLY the ```{} block.\n",
        language, sandbox_check, architecture_plan, lang_lower, code, language, lang_lower,
        lang_lower, lang_lower
    );

    let critic_response = prompt_llm(client, &critic_prompt).await?;
    let refined_code = extract_code(&critic_response, &lang_lower)
        .ok_or_else(|| format!("THE CRITIC failed to produce a valid ```{} block.", language))?;

    Ok(refined_code)
}

// ── Internal: Parsing Utilities ──────────────────────────────────────────────

/// Extract code bounded by ```<language> and ```.
/// Supports any language identifier: ```rust, ```python, ```go, etc.
fn extract_code(response: &str, language: &str) -> Option<String> {
    let fence_open = format!("```{}", language);
    let mut in_block = false;
    let mut code = String::new();

    for line in response.lines() {
        if !in_block && line.starts_with(&fence_open) {
            in_block = true;
            continue;
        } else if in_block && line.starts_with("```") {
            return Some(code);
        }

        if in_block {
            code.push_str(line);
            code.push('\n');
        }
    }

    None
}

/// Extract stdout from a GuardrailResult success JSON.
fn extract_stdout(json_str: &str) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
        if let Some(stdout) = v.get("stdout").and_then(|s| s.as_str()) {
            return Some(stdout.to_string());
        }
    }
    None
}

/// Extract a readable error summary from a GuardrailResult JSON.
fn summarize_error(json_str: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
        let error_type = v
            .get("error_type")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown");
        let phase = v
            .get("phase")
            .and_then(|s| s.as_str())
            .unwrap_or("unknown");
        return format!("[{}::{}]", phase, error_type);
    }
    "Generic Guardrail Error".to_string()
}
