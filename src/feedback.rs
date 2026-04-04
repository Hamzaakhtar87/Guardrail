// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// feedback.rs — Structured JSON output for the LLM feedback loop
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Every execution — success or failure — produces a deterministic JSON envelope
// that the upstream LLM can parse to decide its next action. There are no
// ambiguous string formats here; everything is typed via serde.
//
// SUCCESS envelope:
//   { "status": "success", "stdout": "<captured output>" }
//
// ERROR envelope:
//   { "status": "error", "phase": "compilation|execution",
//     "error_type": "compile_error|timeout|memory_exceeded|trap|runtime",
//     "message": "<diagnostic text>" }
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use serde::Serialize;

// ── Error Classification ─────────────────────────────────────────────────────
// Deterministic enum so the LLM can branch on error type without parsing
// free-text messages.

/// Classifies the phase where the failure occurred.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Failed during `rustc` compilation to `.wasm`.
    Compilation,
    /// Failed during Wasmtime execution of the `.wasm` module.
    Execution,
}

/// Classifies the specific error type within an execution failure.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorType {
    /// `rustc` returned a non-zero exit code.
    CompileError,
    /// Epoch deadline exceeded — the 5-second wall-clock timeout fired.
    Timeout,
    /// StoreLimits denied a memory growth request (50MB ceiling breached).
    MemoryExceeded,
    /// Wasmtime trap (unreachable, div-by-zero, stack overflow, etc).
    Trap,
    /// Catch-all for unexpected runtime errors.
    Runtime,
}

// ── JSON Envelope ────────────────────────────────────────────────────────────
// A single tagged enum that serializes into the exact JSON shape the LLM expects.

/// The top-level feedback envelope returned from every Guardrail invocation.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum GuardrailResult {
    /// The WASM module executed successfully and produced stdout output.
    Success {
        stdout: String,
    },
    /// The pipeline failed at some phase with a classified error.
    Error {
        phase: Phase,
        error_type: ErrorType,
        message: String,
    },
}

impl GuardrailResult {
    // ── Constructors ─────────────────────────────────────────────────────

    /// Build a success result from captured stdout bytes.
    pub fn success(stdout: String) -> Self {
        GuardrailResult::Success { stdout }
    }

    /// Build a compilation error result from rustc stderr.
    pub fn compilation_error(stderr: String) -> Self {
        GuardrailResult::Error {
            phase: Phase::Compilation,
            error_type: ErrorType::CompileError,
            message: stderr,
        }
    }

    /// Build a timeout error result (epoch deadline exceeded).
    pub fn timeout(detail: String) -> Self {
        GuardrailResult::Error {
            phase: Phase::Execution,
            error_type: ErrorType::Timeout,
            message: detail,
        }
    }

    /// Build a memory exceeded error result (StoreLimits denied growth).
    pub fn memory_exceeded(detail: String) -> Self {
        GuardrailResult::Error {
            phase: Phase::Execution,
            error_type: ErrorType::MemoryExceeded,
            message: detail,
        }
    }

    /// Build a trap error result (unreachable, stack overflow, etc).
    pub fn trap(detail: String) -> Self {
        GuardrailResult::Error {
            phase: Phase::Execution,
            error_type: ErrorType::Trap,
            message: detail,
        }
    }

    /// Build a generic runtime error result.
    pub fn runtime_error(detail: String) -> Self {
        GuardrailResult::Error {
            phase: Phase::Execution,
            error_type: ErrorType::Runtime,
            message: detail,
        }
    }

    // ── Serialization ────────────────────────────────────────────────────

    /// Serialize this result to a JSON string.
    /// Panics are impossible here — our types are always serializable.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self)
            .expect("GuardrailResult is always JSON-serializable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_success_json_shape() {
        let result = GuardrailResult::success("Hello, world!".to_string());
        let json: serde_json::Value = serde_json::from_str(&result.to_json()).unwrap();
        assert_eq!(json["status"], "success");
        assert_eq!(json["stdout"], "Hello, world!");
    }

    #[test]
    fn test_error_json_shape() {
        let result = GuardrailResult::timeout("epoch deadline exceeded".to_string());
        let json: serde_json::Value = serde_json::from_str(&result.to_json()).unwrap();
        assert_eq!(json["status"], "error");
        assert_eq!(json["phase"], "execution");
        assert_eq!(json["error_type"], "timeout");
    }
}
