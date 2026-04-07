// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// syndicate.rs — The 10x Coding Syndicate (Phase 8: Multi-File Workspace)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// A language-agnostic "Mixture of Agents" (MoA) coding engine that supports
// both single-file and multi-file project scaffolding:
//
//   Agent 1: THE ARCHITECT
//            Takes the raw task. Outputs a strict directory structure,
//            file manifest, data structures, and algorithm steps. No code.
//
//   Agent 2: THE CODER
//            Takes the Architect's plan. Outputs multiple files using a
//            structured [FILE: path] delimiter format.
//
//   Agent 3: THE CRITIC
//            Takes the Coder's output. Reviews ALL files for correctness,
//            memory safety, and logic. Outputs a refined multi-file version.
//
// WORKSPACE EXECUTION:
//   For Rust: Parses multi-file output → writes to temp directory → runs
//   `cargo build` and `cargo run` natively on the host. Captures stderr/stdout
//   as the feedback mechanism for the remediation loop.
//
//   For other languages: Returns the Critic's final output directly.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

const OLLAMA_ENDPOINT: &str = "http://127.0.0.1:11434/api/generate";
const MODEL_NAME: &str = "qwen2.5-coder:latest";

/// Maximum remediation rounds after the initial Architect→Coder→Critic pass.
const MAX_REMEDIATION_ROUNDS: usize = 3;

/// HTTP timeout for Ollama requests. 5 minutes for M1 quantized inference.
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
/// * `language` — Target language (e.g. "rust", "python", "go").
/// * `expected_output` — Optional expected substring in stdout for logic verification.
///
/// # Behavior
/// - If `language` is "rust": multi-file workspace → `cargo build` → `cargo run`
///   with output verification and remediation on failure.
/// - Otherwise: Architect→Coder→Critic only, returns final output directly.
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

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // AGENT 1: THE ARCHITECT
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    println!("  [SYNDICATE] Agent 1/3: THE ARCHITECT — Generating project blueprint...");

    let architect_prompt = build_architect_prompt(task, language, is_rust);
    let architecture_plan = prompt_llm(&client, &architect_prompt).await?;
    println!(
        "  [SYNDICATE] Architect delivered blueprint ({} bytes).",
        architecture_plan.len()
    );

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // AGENT 2: THE CODER
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    println!("  [SYNDICATE] Agent 2/3: THE CODER — Implementing multi-file project...");

    let coder_prompt = build_coder_prompt(language, &architecture_plan, None, None, None);
    let coder_response = prompt_llm(&client, &coder_prompt).await?;
    let initial_files = parse_multi_file_output(&coder_response);

    if initial_files.is_empty() {
        return Err("THE CODER failed to produce any [FILE: ...] blocks.".to_string());
    }
    println!(
        "  [SYNDICATE] Coder delivered {} file(s): {}",
        initial_files.len(),
        initial_files
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    );

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // AGENT 3: THE CRITIC
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    println!("  [SYNDICATE] Agent 3/3: THE CRITIC — Reviewing all files...");

    let coder_raw_output = format_files_for_prompt(&initial_files);
    let critic_prompt = build_critic_prompt(language, &architecture_plan, &coder_raw_output);
    let critic_response = prompt_llm(&client, &critic_prompt).await?;
    let critic_files = parse_multi_file_output(&critic_response);

    // If the Critic returned files, use them. Otherwise, fall back to Coder's files.
    let mut current_files = if critic_files.is_empty() {
        println!("  [SYNDICATE] Critic returned no [FILE:] blocks — using Coder's output.");
        initial_files
    } else {
        println!(
            "  [SYNDICATE] Critic delivered {} hardened file(s).",
            critic_files.len()
        );
        critic_files
    };

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // WORKSPACE EXECUTION (Rust) or DIRECT RETURN
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    if !is_rust {
        println!(
            "  [SYNDICATE] Language '{}' — Returning Critic's multi-file output directly.",
            language
        );
        return Ok(format_files_for_prompt(&current_files));
    }

    // ── Rust Workspace: cargo build + cargo run remediation loop ──────────
    for round in 0..=MAX_REMEDIATION_ROUNDS {
        let round_label = if round == 0 {
            "Initial submission".to_string()
        } else {
            format!("Remediation {}/{}", round, MAX_REMEDIATION_ROUNDS)
        };
        println!("  [SYNDICATE] Workspace Execution — {}...", round_label);

        // Write files to temp workspace and execute
        let exec_result = tokio::task::spawn_blocking({
            let files = current_files.clone();
            move || execute_workspace(&files)
        })
        .await
        .map_err(|e| format!("Workspace thread panicked: {}", e))?;

        match exec_result {
            Ok(stdout) => {
                // Phase 7: Deterministic Output Verification
                if let Some(expected) = expected_output {
                    let stdout_trimmed = stdout.trim();
                    if stdout_trimmed.contains(expected) {
                        println!(
                            "  [SYNDICATE] ✅ SUCCESS — Output verified on {}.",
                            round_label
                        );
                        return Ok(stdout);
                    } else {
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

                        // Remediate with logic error
                        current_files = remediate(
                            &client,
                            language,
                            &architecture_plan,
                            &current_files,
                            &logic_error,
                        )
                        .await?;
                        continue;
                    }
                } else {
                    // No expected output — build + run success is sufficient.
                    println!(
                        "  [SYNDICATE] ✅ SUCCESS — Workspace built and ran on {}.",
                        round_label
                    );
                    return Ok(stdout);
                }
            }
            Err(build_error) => {
                println!("  [SYNDICATE] ❌ BUILD/RUN FAILURE — {}", &build_error[..build_error.len().min(200)]);

                if round == MAX_REMEDIATION_ROUNDS {
                    return Err(format!(
                        "Syndicate exhausted all {} remediation rounds. Last error: {}",
                        MAX_REMEDIATION_ROUNDS, build_error
                    ));
                }

                // Remediate with build error
                current_files = remediate(
                    &client,
                    language,
                    &architecture_plan,
                    &current_files,
                    &build_error,
                )
                .await?;
            }
        }
    }

    Err("Unreachable".into())
}

// ── Internal: Remediation ────────────────────────────────────────────────────

/// Re-invoke Coder → Critic with error telemetry for remediation.
async fn remediate(
    client: &reqwest::Client,
    language: &str,
    architecture_plan: &str,
    current_files: &HashMap<String, String>,
    error_message: &str,
) -> Result<HashMap<String, String>, String> {
    println!("  [SYNDICATE] Initiating remediation: Coder → Critic with error...");

    // Phase 9: Delta Remediation
    let broken_files = extract_error_files(error_message);
    if !broken_files.is_empty() {
        println!("  [SYNDICATE] Delta Remediation: Implicating specific files: {:?}", broken_files);
    } else {
        println!("  [SYNDICATE] No specific files parsed from error. Falling back to full rewrite.");
    }

    let current_output = format_files_for_prompt(current_files);
    let remediation_prompt = build_coder_prompt(
        language,
        architecture_plan,
        Some(&current_output),
        Some(error_message),
        (!broken_files.is_empty()).then(|| broken_files.as_slice()),
    );

    let remediated_raw = prompt_llm(client, &remediation_prompt).await?;
    let remediated_files = parse_multi_file_output(&remediated_raw);

    if remediated_files.is_empty() {
        return Err("Remediation Coder failed to produce any [FILE:] blocks.".to_string());
    }

    // Run through Critic
    let remediated_output = format_files_for_prompt(&remediated_files);
    let critic_prompt = build_critic_prompt(language, architecture_plan, &remediated_output);
    let critic_response = prompt_llm(client, &critic_prompt).await?;
    let critic_files = parse_multi_file_output(&critic_response);

    let partial_files = if critic_files.is_empty() {
        remediated_files
    } else {
        critic_files
    };

    // Phase 9: Merge Delta
    let mut final_files = current_files.clone();
    for (path, content) in partial_files {
        final_files.insert(path, content);
    }

    println!(
        "  [SYNDICATE] Remediation complete — {} file(s) updated and merged.",
        final_files.len()
    );

    Ok(final_files)
}

// ── Internal: Workspace Execution ────────────────────────────────────────────

/// Write multi-file output to a temp directory, then run `cargo build` and
/// `cargo run`. Returns Ok(stdout) on success, Err(stderr) on failure.
fn execute_workspace(files: &HashMap<String, String>) -> Result<String, String> {
    // Create temp workspace
    let workspace_dir = tempfile::tempdir()
        .map_err(|e| format!("Failed to create temp workspace: {}", e))?;
    let base = workspace_dir.path();

    println!(
        "  [WORKSPACE] Writing {} file(s) to {}",
        files.len(),
        base.display()
    );

    // Write all files
    for (path, content) in files {
        let full_path = base.join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create dir {}: {}", parent.display(), e))?;
        }
        std::fs::write(&full_path, content)
            .map_err(|e| format!("Failed to write {}: {}", path, e))?;
        println!("  [WORKSPACE]   ✓ {}", path);
    }

    // ── cargo build ──────────────────────────────────────────────────────
    println!("  [WORKSPACE] Running: cargo build...");
    let build_output = Command::new("cargo")
        .arg("build")
        .current_dir(base)
        .output()
        .map_err(|e| format!("Failed to spawn `cargo build`: {}", e))?;

    if !build_output.status.success() {
        let stderr = String::from_utf8_lossy(&build_output.stderr).to_string();
        return Err(format!(
            "[Build Error] cargo build failed:\n{}",
            stderr
        ));
    }
    println!("  [WORKSPACE] cargo build — OK");

    // ── cargo run ────────────────────────────────────────────────────────
    println!("  [WORKSPACE] Running: cargo run...");
    let run_output = Command::new("cargo")
        .arg("run")
        .current_dir(base)
        .output()
        .map_err(|e| format!("Failed to spawn `cargo run`: {}", e))?;

    let stdout = String::from_utf8_lossy(&run_output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&run_output.stderr).to_string();

    if !run_output.status.success() {
        return Err(format!(
            "[Runtime Error] cargo run failed:\nstdout: {}\nstderr: {}",
            stdout, stderr
        ));
    }

    println!("  [WORKSPACE] cargo run — OK");
    println!("  [WORKSPACE] stdout: {}", stdout.trim());

    // Keep workspace alive until we're done reading output
    // (TempDir drops and cleans up when this function returns)
    Ok(stdout)
}

// ── Internal: Prompt Builders ────────────────────────────────────────────────

/// Build the Architect prompt — now outputs directory structure + file manifest.
fn build_architect_prompt(task: &str, language: &str, is_rust: bool) -> String {
    let workspace_instructions = if is_rust {
        "The project will be compiled as a Cargo workspace using `cargo build` and `cargo run`.\n\
         You MUST specify the exact Cargo.toml contents requirements (package name, edition, \
         any dependencies if needed). The code runs natively on the host machine (Apple M1).\n\
         You MUST output a DIRECTORY STRUCTURE showing every file that must be created."
    } else {
        "You MUST output a DIRECTORY STRUCTURE showing every file that must be created."
    };

    format!(
        "You are THE ARCHITECT — an elite systems designer specializing in \
        high-performance {}.\n\n\
        Your job is to produce a STRICT PROJECT BLUEPRINT for the following task.\n\n\
        You must output:\n\
        1. DIRECTORY STRUCTURE: A tree listing every file path (e.g., Cargo.toml, src/main.rs, \
           src/handlers.rs).\n\
        2. FILE MANIFEST: For each file, describe its exact purpose and the types/functions \
           it must contain.\n\
        3. DATA STRUCTURES: Exact struct/type definitions with field types.\n\
        4. ALGORITHM: Step-by-step numbered breakdown.\n\
        5. EDGE CASES: Invariants that must be handled.\n\n\
        RULES:\n\
        - Do NOT write any {} code. Only the blueprint.\n\
        - Be extremely specific. Ambiguity kills downstream agents.\n\
        - {}\n\n\
        TASK:\n{}\n",
        language, language, workspace_instructions, task
    )
}

/// Build a Coder prompt for multi-file output.
fn build_coder_prompt(
    language: &str,
    architecture_plan: &str,
    previous_output: Option<&str>,
    error_message: Option<&str>,
    broken_files: Option<&[String]>,
) -> String {
    let file_format_instructions = format!(
        "OUTPUT FORMAT:\n\
         You must output ALL project files using this EXACT delimiter format:\n\n\
         [FILE: Cargo.toml]\n\
         ```toml\n\
         # cargo config here\n\
         ```\n\n\
         [FILE: src/main.rs]\n\
         ```{}\n\
         // code here\n\
         ```\n\n\
         RULES:\n\
         - Output EVERY file specified in the Architect's blueprint.\n\
         - Each file MUST start with [FILE: path/to/file]\n\
         - Each file's content MUST be inside a fenced code block (``` ```).\n\
         - NO explanations. NO commentary outside the file blocks.\n\
         - Use ONLY the standard library unless the Architect explicitly specifies dependencies.",
        language.to_lowercase()
    );

    let delta_instructions = if let Some(files) = broken_files {
        format!(
            "DELTA REMEDIATION:\n\
             ONLY rewrite the following files: {:?}.\n\
             DO NOT output [FILE: ...] blocks for files that are not in this list.\n\
             Assume the other files are perfect.\n\n",
            files
        )
    } else {
        String::new()
    };

    match (previous_output, error_message) {
        (Some(prev), Some(error)) => format!(
            "You are THE CODER — an elite {} implementation engineer.\n\n\
            You previously wrote the following project files:\n\
            ─────────────────────────────────────────────\n\
            {}\n\
            ─────────────────────────────────────────────\n\n\
            The workspace build/run FAILED with this error:\n\
            ```\n{}\n```\n\n\
            The original ARCHITECT's blueprint was:\n\
            ─────────────────────────────────────────────\n\
            {}\n\
            ─────────────────────────────────────────────\n\n\
            CRITICAL CONTEXT ANCHORING:\n\
            Read the compiler errors carefully. The errors specify EXACTLY which file failed (e.g., --> src/handlers.rs:3:32).\n\
            You MUST map that error to the correct [FILE: ...] block.\n\
            Do NOT put imports or fixes in src/main.rs if the error explicitly occurred in src/handlers.rs.\n\
            Check your file boundaries.\n\n\
            {}\
            Fix ALL reported errors.\n\n\
            {}\n",
            language, prev, error, architecture_plan, delta_instructions, file_format_instructions
        ),
        _ => format!(
            "You are THE CODER — an elite {} implementation engineer. You write flawless, \
            production-grade {} code from architectural blueprints.\n\n\
            Below is the project blueprint from THE ARCHITECT. You must implement it \
            EXACTLY as specified.\n\n\
            ARCHITECT'S BLUEPRINT:\n\
            ─────────────────────────────────────────────\n\
            {}\n\
            ─────────────────────────────────────────────\n\n\
            {}\n",
            language, language, architecture_plan, file_format_instructions
        ),
    }
}

/// Build the Critic prompt for multi-file review.
fn build_critic_prompt(
    language: &str,
    architecture_plan: &str,
    coder_output: &str,
) -> String {
    format!(
        "You are THE CRITIC — an adversarial code reviewer with zero tolerance for defects.\n\n\
        Below is a multi-file {} project written by THE CODER based on an architectural blueprint.\n\
        Your job is to AGGRESSIVELY review ALL files and output a REFINED, FINAL version.\n\n\
        REVIEW CHECKLIST:\n\
        1. CORRECTNESS: Does every file match the Architect's specification?\n\
        2. MEMORY SAFETY: Any panics, overflows, or resource leaks?\n\
        3. LOGIC FLAWS: Off-by-one errors, incorrect handling, missing edge cases?\n\
        4. INTER-FILE CONSISTENCY: Do module imports, function signatures, and types align?\n\
        5. BUILD COMPLIANCE: Will `cargo build` succeed? Are all modules declared?\n\
        6. OUTPUT: Does the program produce the expected stdout when run?\n\
        7. CROSS-FILE VERIFICATION: If src/main.rs calls a function in src/handlers.rs, ensure the module is publicly exported (`pub mod handlers;`) and that ALL necessary imports (like `TcpStream` or `io::Write`) are present in the EXACT file where they are used, not just in main.\n\n\
        ARCHITECT'S BLUEPRINT:\n\
        ─────────────────────────────────────────────\n\
        {}\n\
        ─────────────────────────────────────────────\n\n\
        CODER'S IMPLEMENTATION:\n\
        ─────────────────────────────────────────────\n\
        {}\n\
        ─────────────────────────────────────────────\n\n\
        OUTPUT FORMAT:\n\
        Return ALL files using the [FILE: path] delimiter format:\n\n\
        [FILE: path/to/file]\n\
        ```{}\n\
        // refined code\n\
        ```\n\n\
        - Output EVERY file, even if unchanged.\n\
        - NO explanations. NO commentary outside file blocks.\n",
        language, architecture_plan, coder_output, language.to_lowercase()
    )
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

// ── Internal: Multi-File Parser ──────────────────────────────────────────────

/// Parse the LLM's multi-file output into a HashMap of filepath → content.
///
/// Expected format:
/// ```text
/// [FILE: Cargo.toml]
/// ```toml
/// [package]
/// name = "myproject"
/// ```
///
/// [FILE: src/main.rs]
/// ```rust
/// fn main() { }
/// ```
/// ```
fn parse_multi_file_output(response: &str) -> HashMap<String, String> {
    let mut files: HashMap<String, String> = HashMap::new();
    let mut current_path: Option<String> = None;
    let mut current_content = String::new();
    let mut in_code_block = false;

    for line in response.lines() {
        // Detect [FILE: path/to/file] markers
        if line.starts_with("[FILE:") && line.ends_with(']') {
            // Save previous file if we have one
            if let Some(path) = current_path.take() {
                let content = current_content.trim().to_string();
                if !content.is_empty() {
                    files.insert(path, content);
                }
            }
            // Extract the new file path
            let path = line
                .trim_start_matches("[FILE:")
                .trim_end_matches(']')
                .trim()
                .to_string();
            current_path = Some(path);
            current_content = String::new();
            in_code_block = false;
            continue;
        }

        // Only capture content when we have an active file path
        if current_path.is_some() {
            if line.starts_with("```") {
                if in_code_block {
                    // Closing fence — stop capturing for this block
                    in_code_block = false;
                } else {
                    // Opening fence — start capturing (skip the fence line itself)
                    in_code_block = true;
                }
                continue;
            }

            if in_code_block {
                current_content.push_str(line);
                current_content.push('\n');
            }
        }
    }

    // Save the last file
    if let Some(path) = current_path {
        let content = current_content.trim().to_string();
        if !content.is_empty() {
            files.insert(path, content);
        }
    }

    files
}

/// Format a file map back into the [FILE: ...] format for prompts.
fn format_files_for_prompt(files: &HashMap<String, String>) -> String {
    let mut output = String::new();
    // Sort keys for deterministic output
    let mut keys: Vec<&String> = files.keys().collect();
    keys.sort();
    for path in keys {
        let content = &files[path];
        let lang = infer_fence_language(path);
        output.push_str(&format!("[FILE: {}]\n```{}\n{}\n```\n\n", path, lang, content));
    }
    output
}

/// Infer the markdown fence language from a file extension.
fn infer_fence_language(path: &str) -> &str {
    if path.ends_with(".rs") {
        "rust"
    } else if path.ends_with(".toml") {
        "toml"
    } else if path.ends_with(".py") {
        "python"
    } else if path.ends_with(".go") {
        "go"
    } else if path.ends_with(".js") {
        "javascript"
    } else if path.ends_with(".ts") {
        "typescript"
    } else if path.ends_with(".json") {
        "json"
    } else if path.ends_with(".yaml") || path.ends_with(".yml") {
        "yaml"
    } else {
        ""
    }
}

// ── Internal: Delta Remediation ──────────────────────────────────────────────

/// Phase 9: Parse error output for `--> src/handlers.rs` to extract implicated files.
fn extract_error_files(error_log: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in error_log.lines() {
        if let Some(idx) = line.find("--> ") {
            let path_part = &line[idx + 4..];
            // Path ends at the first colon (e.g. src/handlers.rs:3:32)
            let path = path_part.split(':').next().unwrap_or("").trim().to_string();
            if !path.is_empty() && !files.contains(&path) {
                files.push(path);
            }
        }
    }
    files
}
