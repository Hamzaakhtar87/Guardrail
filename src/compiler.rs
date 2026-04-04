// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// compiler.rs — Source code → .wasm compilation via rustc subprocess
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Takes a raw Rust source string, writes it to a temp file, and invokes
// `rustc --target wasm32-wasip1` to produce a .wasm binary.
//
// DESIGN DECISIONS:
//   - We use `rustc` directly (not `cargo`) to avoid the overhead of a full
//     Cargo project scaffold for each LLM-generated snippet. Single-file
//     compilation is sufficient for the sandboxed execution model.
//   - The TempDir is returned alongside the .wasm path so the caller controls
//     the lifetime of the compiled artifact. When the TempDir is dropped,
//     all compilation artifacts are cleaned up automatically.
//   - We compile in release mode (--opt-level=2) to produce smaller, faster
//     WASM modules. The LLM code is untrusted and runs sandboxed, so debug
//     info is unnecessary overhead.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use tempfile::TempDir;

/// The result of a successful compilation: the path to the .wasm artifact
/// and the TempDir that owns it. The TempDir must be kept alive as long as
/// the .wasm file is needed.
pub struct CompilationArtifact {
    /// Absolute path to the compiled .wasm module.
    pub wasm_path: PathBuf,
    /// Owning handle for the temporary directory. When this is dropped,
    /// the .wasm file and all intermediates are cleaned up.
    _temp_dir: TempDir,
}

/// Compile a raw Rust source string into a WASM module targeting `wasm32-wasip1`.
///
/// # Arguments
/// * `source_code` — The complete Rust source code to compile (must contain `fn main()`).
///
/// # Returns
/// * `Ok(CompilationArtifact)` — The compiled .wasm file and its owning temp directory.
/// * `Err(anyhow::Error)` — Contains the rustc stderr output for the LLM feedback loop.
pub fn compile_to_wasm(source_code: &str) -> Result<CompilationArtifact> {
    // ── Step 1: Create an isolated temp directory for this compilation ────
    // Each compilation gets its own directory to prevent interference between
    // concurrent sandbox invocations.
    let temp_dir = TempDir::new()
        .context("Failed to create temporary directory for compilation")?;

    let source_path = temp_dir.path().join("main.rs");
    let wasm_path = temp_dir.path().join("output.wasm");

    // ── Step 2: Write the LLM-generated source code to disk ──────────────
    std::fs::write(&source_path, source_code)
        .context("Failed to write source code to temp file")?;

    // ── Step 3: Invoke rustc with the wasm32-wasip1 target ───────────────
    // Flags:
    //   --target wasm32-wasip1  : Compile for WASI Preview 1
    //   --edition 2021          : Use Rust 2021 edition (stable, modern)
    //   -O                      : Optimize for size/speed (release mode)
    //   -o output.wasm          : Output path for the compiled module
    let output = Command::new("rustc")
        .arg("--target")
        .arg("wasm32-wasip1")
        .arg("--edition")
        .arg("2021")
        .arg("-O")
        .arg("-o")
        .arg(&wasm_path)
        .arg(&source_path)
        .output()
        .context("Failed to spawn rustc — is Rust installed and on $PATH?")?;

    // ── Step 4: Check compilation result ─────────────────────────────────
    if !output.status.success() {
        // Extract stderr from rustc — this is the diagnostic text that will
        // be fed back to the LLM so it can correct its code.
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        anyhow::bail!(
            "Compilation failed:\n{}",
            if stderr.is_empty() {
                format!("rustc exited with status {}", output.status)
            } else {
                stderr
            }
        );
    }

    // ── Step 5: Verify the .wasm file was actually produced ──────────────
    if !wasm_path.exists() {
        anyhow::bail!(
            "rustc succeeded but no .wasm file was produced at {}",
            wasm_path.display()
        );
    }

    Ok(CompilationArtifact {
        wasm_path,
        _temp_dir: temp_dir,
    })
}
