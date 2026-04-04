// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// main.rs — Guardrail CLI entry point and demo harness
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// This is the top-level orchestrator for the Guardrail execution sandbox.
// It demonstrates the full pipeline:
//
//   1. LLM source code → rustc → .wasm    (compiler.rs)
//   2. .wasm → Wasmtime sandbox             (sandbox.rs)
//   3. Result → Structured JSON             (feedback.rs)
//
// Three test vectors are exercised:
//   ✓ SUCCESS:  A simple println program → captured stdout
//   ✗ TIMEOUT:  An infinite loop → killed after 5 seconds
//   ✗ OOM:      Excessive allocation → denied by StoreLimits
//   ✗ COMPILE:  Invalid syntax → rustc error feedback
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

mod compiler;
mod feedback;
mod sandbox;

use feedback::GuardrailResult;

fn main() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║           GUARDRAIL — WASM Execution Sandbox                ║");
    println!("║     Deterministic Enforcement Layer for AI Swarm            ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // ── THE ASSAULT VECTOR ───────────────────────────────────────────────
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

    // ── Test Vector 2: Compilation Failure ───────────────────────────────
    // Invalid Rust syntax — rustc will reject this with diagnostics that
    // get fed back to the LLM in the error JSON.
    run_test(
        "COMPILE ERROR — Invalid syntax",
        r#"
fn main() {
    let x = ;  // syntax error: expected expression
    println!("{}", x);
}
"#,
    );

    // ── Test Vector 3: Infinite Loop (Timeout) ───────────────────────────
    // This program enters an infinite loop. The epoch watchdog thread will
    // fire after 5 seconds and kill the execution with an EpochInterruption.
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

    // ── Test Vector 4: Memory Exceeded ───────────────────────────────────
    // Attempts to allocate well beyond the 50MB limit. StoreLimits will deny
    // the memory.grow operation, causing the allocation to fail.
    run_test(
        "MEMORY EXCEEDED — Allocation beyond 50MB",
        r#"
fn main() {
    // Attempt to allocate ~100MB — far beyond the 50MB sandbox limit.
    let mut data: Vec<u8> = Vec::new();
    for _ in 0..100 {
        // Each push of 1MB. StoreLimits will deny growth past 50MB.
        let chunk = vec![42u8; 1024 * 1024];
        data.extend_from_slice(&chunk);
    }
    println!("Allocated {} bytes", data.len());
}
"#,
    );

    println!();
    println!("════════════════════════════════════════════════════════════════");
    println!("  All test vectors complete. Guardrail operational.");
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
