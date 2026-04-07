// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// main.rs — Guardrail CLI entry point and demo harness
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// This is the top-level orchestrator for the Guardrail execution sandbox
// and the mHTTP data ingestion engine. It demonstrates the full pipeline:
//
//   1. LLM source code → rustc → .wasm    (compiler.rs)
//   2. .wasm → Wasmtime sandbox             (sandbox.rs)
//   3. Result → Structured JSON             (feedback.rs)
//   4. URL → raw HTML → compressed text     (mhttp.rs)
//   5. Task/Prompt → MoA coding engine   (syndicate.rs)
//   6. Web3 Dev Eval → mHTTP + LLM          (scout.rs)
//   7. Mixture of Agents coding engine       (syndicate.rs)
//   8. Concurrent cohort evaluation          (swarm.rs)
//
// Test vectors exercised:
//   ✓ WASM SANDBOX: Hostile reconnaissance, compile errors, timeout, OOM
//   ✓ mHTTP:        Wikipedia article fetch + DOM compression audit
//   ✓ SYNDICATE:    Autonomous MoA code generation & error remediation
//   ✓ SCOUT:        Automated Web3/Rust developer technical evaluation
//   ✓ SYNDICATE:    Mixture of Agents + Deterministic Output Verification
//   ✓ SWARM:        Concurrent evaluation of cohort identities
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

mod compiler;
mod feedback;
mod mhttp;
mod sandbox;
mod scout;
mod swarm;
mod syndicate;

use feedback::GuardrailResult;

#[tokio::main]
async fn main() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║           GUARDRAIL — Execution Sandbox + mHTTP Engine      ║");
    println!("║     Deterministic Enforcement Layer for AI Swarm            ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // ── WASM Sandbox Tests ───────────────────────────────────────────────
    // These run on a dedicated blocking thread because Wasmtime's synchronous
    // WASI implementation uses internal block_on() calls that conflict with
    // tokio's async runtime. spawn_blocking moves them off the async executor.
    tokio::task::spawn_blocking(|| {
        // ── THE ASSAULT VECTOR ───────────────────────────────────────────
        // Testing the WASI zero-trust boundaries (Env Vars, File System, Network)
        run_test(
            "HOSTILE RECONNAISSANCE — Bypassing WASI Zero-Trust",
            r#"
use std::env;
use std::fs::File;
use std::io::Read;
use std::net::TcpStream;

fn main() {
    println!("[ATTACK] Initiating sandbox reconnaissance...");

    // Attack 1: Environment Variable Theft
    println!("\n[ATTACK] 1. Dumping environment variables...");
    let mut env_count = 0;
    for (key, value) in env::vars() {
        println!("{}: {}", key, value);
        env_count += 1;
    }
    if env_count == 0 {
        println!("DEFENDED: No environment variables leaked.");
    }

    // Attack 2: File System Breakout
    println!("\n[ATTACK] 2. Attempting to read host file system (/etc/passwd)...");
    match File::open("/etc/passwd") {
        Ok(mut file) => {
            let mut contents = String::new();
            file.read_to_string(&mut contents).unwrap();
            println!("CRITICAL FAILURE: File system breached! Read {} bytes.", contents.len());
        }
        Err(e) => println!("DEFENDED: File system access denied: {}", e),
    }

    // Attack 3: Network Exfiltration
    println!("\n[ATTACK] 3. Attempting outbound network connection to 8.8.8.8...");
    match TcpStream::connect("8.8.8.8:80") {
        Ok(_) => println!("CRITICAL FAILURE: Network breached! Socket opened."),
        Err(e) => println!("DEFENDED: Network access denied: {}", e),
    }
}
"#,
        );

        // ── Test Vector 2: Compilation Failure ───────────────────────────
        run_test(
            "COMPILE ERROR — Invalid syntax",
            r#"
fn main() {
    let x = ;  // syntax error: expected expression
    println!("{}", x);
}
"#,
        );

        // ── Test Vector 3: Infinite Loop (Timeout) ───────────────────────
        // NOTE: This test takes ~5 seconds to complete by design.
        run_test(
            "TIMEOUT — Infinite loop (5s deadline)",
            r#"
fn main() {
    println!("Starting infinite loop...");
    loop {
        // Spin forever — the epoch watchdog will kill this.
    }
}
"#,
        );

        // ── Test Vector 4: Memory Exceeded ───────────────────────────────
        run_test(
            "MEMORY EXCEEDED — Allocation beyond 50MB",
            r#"
fn main() {
    let mut data: Vec<u8> = Vec::new();
    for _ in 0..100 {
        let chunk = vec![42u8; 1024 * 1024];
        data.extend_from_slice(&chunk);
    }
    println!("Allocated {} bytes", data.len());
}
"#,
        );
    }).await.expect("WASM sandbox tests panicked");

    // ── mHTTP ASSAULT VECTORS ───────────────────────────────────────────────
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║           mHTTP — HOSTILE RECONNAISSANCE TESTS               ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // Attack 1: The Blackhole (Testing the 15s timeout constraint)
    // 10.255.255.1 is non-routable and will force a connection timeout.
    run_mhttp_test(
        "ATTACK 1: Network Timeout (Blackhole IP)",
        "http://10.255.255.1",
    ).await;

    // Attack 2: The Payload Bomb (Testing the 10MB memory guard)
    // Attempting to ingest a 100MB test file to see if the engine catches it before downloading.
    run_mhttp_test(
        "ATTACK 2: Payload Bomb (100MB Zip File)",
        "http://ipv4.download.thinkbroadband.com/100MB.zip",
    ).await;

    // Attack 3: The Dead End (Testing graceful HTTP error handling)
    run_mhttp_test(
        "ATTACK 3: HTTP 404 (Not Found)",
        "https://httpstat.us/404",
    ).await;

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // PHASE 3: The 10x Coding Syndicate — Multi-File Workspace
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  SYNDICATE — 10x Coding Engine (Multi-File Workspace)      ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
    println!("  Architect → Coder → Critic → cargo build → cargo run → Verify");
    println!("  Language: Rust | Task: Multi-file HTTP server (std::net only)");
    println!();

    let syndicate_task = "Scaffold a basic Rust HTTP server project using ONLY the standard \
                          library (no external web frameworks, just std::net). The project must \
                          include: a Cargo.toml (with no external dependencies), a src/main.rs \
                          that binds to 127.0.0.1:0 (OS-assigned port), accepts exactly ONE \
                          incoming TCP connection, delegates it to a handler, prints the bound \
                          port to stdout in the format 'Listening on port XXXX', prints \
                          'Connection handled' after handling one request, then exits cleanly. \
                          It must also include a src/handlers.rs module that contains a function \
                          to write a valid HTTP 200 OK response with body 'Hello from Guardrail' \
                          to the TcpStream. The main.rs must declare `mod handlers;` and use it.";

    match syndicate::syndicate_coding_loop(syndicate_task, "rust", Some("Connection handled")).await {
        Ok(output) => {
            println!("┌─────────────────────────────────────────────────────────────┐");
            println!("│ SYNDICATE SUCCESS: Multi-file project built and verified    │");
            println!("└─────────────────────────────────────────────────────────────┘");
            println!("  Output: {}", output.trim());
        }
        Err(e) => {
            println!("┌─────────────────────────────────────────────────────────────┐");
            println!("│ SYNDICATE FAILURE: MoA pipeline exhausted                   │");
            println!("└─────────────────────────────────────────────────────────────┘");
            println!("  Error: {}", e);
        }
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // PHASE 4: Scout Agent — Web3 Developer Evaluation
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    println!();
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║           SCOUT — Autonomous Web3 Developer Evaluation       ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
    
    // Evaluate the official Rust repository README as a known target
    let target = "https://github.com/rust-lang/rust/blob/master/README.md";
    
    match scout::evaluate_candidate(target).await {
        Ok(assessment) => {
            println!("┌─────────────────────────────────────────────────────────────┐");
            println!("│ SCOUT SUCCESS: Target Evaluated and Formatted               │");
            println!("└─────────────────────────────────────────────────────────────┘");
            println!("  [SCORE]: {}/10", assessment.developer_score);
            println!("  [RATIONALE]: {}", assessment.summary_rationale);
            println!("  [STRENGTHS]:");
            for strength in assessment.technical_strengths {
                println!("    - {}", strength);
            }
        }
        Err(e) => {
            println!("┌─────────────────────────────────────────────────────────────┐");
            println!("│ SCOUT FAILURE: Evaluation Failed                            │");
            println!("└─────────────────────────────────────────────────────────────┘");
            println!("  Error: {}", e);
        }
    }

    // Evaluate an adversarial "Garbage" target (Wikipedia - Banana)
    println!();
    println!("  [SCOUT] Initiating adversarial evaluation vector...");
    let adversarial_target = "https://en.wikipedia.org/wiki/Banana";

    match scout::evaluate_candidate(adversarial_target).await {
        Ok(assessment) => {
            println!("┌─────────────────────────────────────────────────────────────┐");
            println!("│ SCOUT ADVERSARIAL SUCCESS: Handled Irrelevant Garbage       │");
            println!("└─────────────────────────────────────────────────────────────┘");
            println!("  [SCORE]: {}/10", assessment.developer_score);
            println!("  [RATIONALE]: {}", assessment.summary_rationale);
            println!("  [STRENGTHS]:");
            for strength in assessment.technical_strengths {
                println!("    - {}", strength);
            }
        }
        Err(e) => {
            println!("┌─────────────────────────────────────────────────────────────┐");
            println!("│ SCOUT ADVERSARIAL FAILURE: Parser broke on garbage          │");
            println!("└─────────────────────────────────────────────────────────────┘");
            println!("  Error: {}", e);
        }
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // PHASE 5: Swarm Parallelization
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    println!();
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║           SWARM — Concurrent Cohort Evaluation               ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    let cohort = vec![
        "https://github.com/rust-lang/rust/blob/master/README.md",
        "http://10.255.255.1",
        "http://ipv4.download.thinkbroadband.com/100MB.zip",
        "https://httpstat.us/404",
    ];

    let results = swarm::evaluate_cohort(cohort).await;

    println!();
    println!("┌─────────────────────────────────────────────────────────────┐");
    println!("│ SWARM BATCH COMPLETE: Processed {} targets concurrently      │", results.len());
    println!("└─────────────────────────────────────────────────────────────┘");
    
    for (url, res) in results {
        println!("  TARGET: {}", url);
        match res {
            Ok(eval) => {
                println!("    [SCORE]: {}/10", eval.developer_score);
                println!("    [RATIONALE]: {}", eval.summary_rationale);
            }
            Err(e) => {
                println!("    [ERROR]: {}", e);
            }
        }
        println!("  ─────────────────────────────────────────────────────────");
    }

    println!();
    println!("════════════════════════════════════════════════════════════════");
    println!("  All test vectors complete. Guardrail + mHTTP + Swarm operational.");
    println!("════════════════════════════════════════════════════════════════");
}

// ── Pipeline Orchestrator ────────────────────────────────────────────────────

/// Run a single test vector through the full Guardrail pipeline.
///
/// Steps:
/// 1. Compile the source code to .wasm via rustc
/// 2. Execute the .wasm in the sandboxed Wasmtime engine
/// 3. Format the result as structured JSON
/// 4. Print the JSON output
fn run_test(label: &str, source_code: &str) {
    println!("┌─────────────────────────────────────────────────────────────┐");
    println!("│ TEST: {:<53}│", label);
    println!("└─────────────────────────────────────────────────────────────┘");

    let start = std::time::Instant::now();

    // ── Phase 1: Compile ─────────────────────────────────────────────────
    let artifact = match compiler::compile_to_wasm(source_code) {
        Ok(artifact) => artifact,
        Err(e) => {
            // Compilation failed — format as JSON error and return.
            let result = GuardrailResult::compilation_error(format!("{:#}", e));
            print_result(&result, start.elapsed());
            return;
        }
    };

    // ── Phase 2: Execute in sandbox ──────────────────────────────────────
    let result = sandbox::execute_wasm(&artifact.wasm_path);

    // ── Phase 3: Output structured JSON ──────────────────────────────────
    print_result(&result, start.elapsed());
}

/// Print a GuardrailResult as formatted JSON with timing metadata.
fn print_result(result: &GuardrailResult, elapsed: std::time::Duration) {
    println!("  ⏱  Elapsed: {:.2}s", elapsed.as_secs_f64());
    println!("  📋 JSON Output:");
    println!("  ─────────────────────────────────────────────────────────");
    // Indent each line of the JSON for visual clarity in terminal output.
    for line in result.to_json().lines() {
        println!("    {}", line);
    }
    println!("  ─────────────────────────────────────────────────────────");
    println!();
}

// ── mHTTP Test Orchestrator ─────────────────────────────────────────────────

/// Run a single mHTTP test vector: fetch a URL, strip the DOM, and report
/// the compression ratio (raw HTML bytes vs compressed text bytes).
async fn run_mhttp_test(label: &str, url: &str) {
    println!("┌─────────────────────────────────────────────────────────────┐");
    println!("│ mHTTP: {:<52}│", label);
    println!("└─────────────────────────────────────────────────────────────┘");

    let start = std::time::Instant::now();

    // ── Phase 1: Fetch raw payload ───────────────────────────────────────
    let (raw_html, raw_size) = match mhttp::fetch_raw_payload(url).await {
        Ok(result) => result,
        Err(e) => {
            println!("  ❌ FETCH FAILED: {}", e);
            println!("  ⏱  Elapsed: {:.2}s", start.elapsed().as_secs_f64());
            println!();
            return;
        }
    };

    // ── Phase 2: Compress ────────────────────────────────────────────────
    let (compressed_text, compressed_size) = match mhttp::compress_html(&raw_html) {
        Ok(result) => result,
        Err(e) => {
            println!("  ❌ COMPRESSION FAILED: {}", e);
            println!("  ⏱  Elapsed: {:.2}s", start.elapsed().as_secs_f64());
            println!();
            return;
        }
    };

    let elapsed = start.elapsed();

    // ── Phase 3: Token efficiency audit ──────────────────────────────────
    let reduction_pct = if raw_size > 0 {
        ((1.0 - (compressed_size as f64 / raw_size as f64)) * 100.0) as u32
    } else {
        0
    };

    // Estimate token counts (rough heuristic: ~4 chars per token for English)
    let raw_tokens_est = raw_size / 4;
    let compressed_tokens_est = compressed_size / 4;

    println!("  ⏱  Elapsed: {:.2}s", elapsed.as_secs_f64());
    println!("  📊 Compression Audit:");
    println!("  ─────────────────────────────────────────────────────────");
    println!("    Source URL:          {}", url);
    println!("    Raw HTML:            {} bytes (~{} tokens)", raw_size, raw_tokens_est);
    println!("    Compressed Text:     {} bytes (~{} tokens)", compressed_size, compressed_tokens_est);
    println!("    Reduction:           {}%", reduction_pct);
    println!("    Tokens Saved:        ~{}", raw_tokens_est.saturating_sub(compressed_tokens_est));
    println!("  ─────────────────────────────────────────────────────────");
    println!("  📋 First 500 chars of compressed output:");
    println!("  ─────────────────────────────────────────────────────────");
    let preview: String = compressed_text.chars().take(500).collect();
    for line in preview.lines() {
        println!("    {}", line);
    }
    println!("    ...");
    println!("  ─────────────────────────────────────────────────────────");
    println!();
}
