// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// sandbox.rs — Wasmtime execution engine with deterministic resource caps
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// This is the enforcement core of Guardrail. It loads a compiled .wasm module
// into a Wasmtime instance with strict, non-negotiable resource limits:
//
//   TIMEOUT:  5 seconds wall-clock (epoch interruption)
//   MEMORY:   50 MB maximum linear memory (StoreLimits)
//   INSTANCES: 1 (no spawning sub-instances)
//   TABLES:   10,000 elements max
//
// SECURITY POSTURE (Zero Trust):
//   - NO filesystem access
//   - NO network access
//   - NO environment variables
//   - NO command-line arguments
//   - ONLY stdout and stderr are captured via in-memory pipes
//
// TIMEOUT MECHANISM:
//   Wasmtime's epoch interruption is used instead of fuel-based metering.
//   Epochs are lighter weight (a single atomic counter check per loop
//   iteration / function call) and are driven by an external timer thread
//   that increments the engine's epoch after 5 seconds. When the epoch
//   fires, the guest traps with an `EpochInterruption` error.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::p1::WasiP1Ctx;
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::WasiCtxBuilder;

use crate::feedback::GuardrailResult;

// ── Constants ────────────────────────────────────────────────────────────────
// All resource caps are defined here as constants for deterministic enforcement.
// These are NOT configurable at runtime — they are architectural invariants.

/// Maximum wall-clock execution time before the epoch watchdog kills the guest.
const TIMEOUT_SECONDS: u64 = 5;

/// Maximum linear memory the WASM module can allocate (50 MB).
/// This caps `memory.grow` operations. If the guest requests more, the
/// growth returns -1 (failure), which typically causes the guest to trap.
const MEMORY_LIMIT_BYTES: usize = 50 * 1024 * 1024; // 50 MB

/// Maximum number of WASM instances that can be created within this store.
/// Set to 1 — the guest cannot spawn sub-instances.
const INSTANCE_LIMIT: usize = 1;

/// Maximum number of table elements (function pointers, etc).
const TABLE_ELEMENTS_LIMIT: usize = 10_000;

// ── Host State ───────────────────────────────────────────────────────────────
// The Store<T> in Wasmtime requires a user-defined state type. Ours holds the
// WASI context and the resource limiter.

/// Host-side state passed into the Wasmtime Store.
/// Contains the WASI P1 context (for stdout/stderr capture) and resource limits.
struct HostState {
    /// WASI Preview 1 context — configured with zero-trust permissions.
    wasi: WasiP1Ctx,
    /// Resource limiter enforcing the 50MB memory ceiling.
    limits: StoreLimits,
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Execute a compiled .wasm module inside the sandboxed Wasmtime engine.
///
/// This function:
/// 1. Creates a Wasmtime Engine with epoch interruption enabled
/// 2. Configures StoreLimits (50MB memory, 1 instance, 10K table elements)
/// 3. Sets up WASI with piped stdout/stderr (zero trust — no FS, no network)
/// 4. Spawns a watchdog thread for the 5-second timeout
/// 5. Loads, links, and instantiates the module
/// 6. Calls `_start` (the WASI entry point)
/// 7. Returns a `GuardrailResult` with captured stdout or classified error
///
/// # Arguments
/// * `wasm_path` — Path to the compiled .wasm file on disk.
///
/// # Returns
/// A `GuardrailResult` — always. This function does NOT propagate errors;
/// every failure mode is captured and classified into the JSON envelope.
pub fn execute_wasm(wasm_path: &Path) -> GuardrailResult {
    match execute_wasm_inner(wasm_path) {
        Ok(stdout) => GuardrailResult::success(stdout),
        Err(e) => classify_execution_error(e),
    }
}

// ── Internal Implementation ──────────────────────────────────────────────────

/// Inner execution function that returns Result for ergonomic error handling.
/// All errors are caught by the outer `execute_wasm` and classified.
fn execute_wasm_inner(wasm_path: &Path) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // ── Step 1: Configure the Wasmtime Engine ────────────────────────────
    // Cranelift is the default compiler backend. We enable epoch interruption
    // so our watchdog thread can kill long-running guests.
    let mut config = Config::new();
    config.epoch_interruption(true);

    let engine = Engine::new(&config)
        .map_err(|e| format!("Failed to create Wasmtime engine: {e}"))?;

    // ── Step 2: Build resource limits ────────────────────────────────────
    // StoreLimits enforce hard caps on memory allocation, instance count,
    // and table element count. These are checked synchronously on every
    // `memory.grow` or `table.grow` operation.
    let limits = StoreLimitsBuilder::new()
        .memory_size(MEMORY_LIMIT_BYTES)
        .instances(INSTANCE_LIMIT)
        .table_elements(TABLE_ELEMENTS_LIMIT)
        .build();

    // ── Step 3: Configure WASI with zero-trust permissions ───────────────
    // The guest can ONLY write to stdout and stderr. Everything else is
    // denied by default (WasiCtxBuilder starts with empty permissions).
    //
    // We pipe stdout and stderr into in-memory buffers so we can capture
    // the output after execution completes.
    let stdout_pipe = MemoryOutputPipe::new(1024 * 1024); // 1MB capacity
    let stderr_pipe = MemoryOutputPipe::new(1024 * 1024);

    let wasi_ctx = WasiCtxBuilder::new()
        .stdout(stdout_pipe.clone())
        .stderr(stderr_pipe.clone())
        // NOTE: No .preopened_dir(), no .env(), no .args(), no .inherit_*()
        // This is a completely isolated execution cell.
        .build_p1();

    // ── Step 4: Create the Store with host state ─────────────────────────
    // The Store<HostState> owns the WASI context and resource limiter.
    let host_state = HostState {
        wasi: wasi_ctx,
        limits,
    };

    let mut store = Store::new(&engine, host_state);

    // Register the resource limiter with the store.
    // The closure returns a mutable reference to the StoreLimits inside
    // our HostState, which Wasmtime calls on every memory/table growth.
    store.limiter(|state| &mut state.limits);

    // Set the epoch deadline to 1 tick. When the engine's epoch counter
    // reaches this value, the guest traps with EpochInterruption.
    store.set_epoch_deadline(1);

    // ── Step 5: Load and compile the .wasm module ────────────────────────
    // Module::from_file reads the .wasm binary and compiles it via
    // Cranelift into native machine code for the host architecture.
    let module = Module::from_file(&engine, wasm_path)
        .map_err(|e| format!("Failed to load/compile .wasm module: {e}"))?;

    // ── Step 6: Link WASI imports ────────────────────────────────────────
    // The Linker resolves the `wasi_snapshot_preview1` imports that the
    // compiled module expects (fd_write, proc_exit, etc).
    let mut linker: Linker<HostState> = Linker::new(&engine);
    wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |state: &mut HostState| {
        &mut state.wasi
    })
    .map_err(|e| format!("Failed to link WASI imports: {e}"))?;

    // ── Step 7: Spawn the timeout watchdog thread ────────────────────────
    // This thread sleeps for TIMEOUT_SECONDS, then increments the engine's
    // epoch counter. Since we set the deadline to 1, this causes the guest
    // to trap on the very next epoch check after the timeout.
    //
    // We use Arc<Engine> so the watchdog thread can safely reference the
    // engine without lifetime issues.
    let engine_arc = Arc::new(engine.clone());
    let watchdog = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(TIMEOUT_SECONDS));
        engine_arc.increment_epoch();
    });

    // ── Step 8: Instantiate and execute ──────────────────────────────────
    // `_start` is the standard WASI entry point, equivalent to `main()`.
    // We instantiate the module, look up `_start`, and call it.
    let instance = linker.instantiate(&mut store, &module)
        .map_err(|e| format!("Failed to instantiate WASM module: {e}"))?;

    let start_func = instance
        .get_typed_func::<(), ()>(&mut store, "_start")
        .map_err(|e| format!("Module does not export a `_start` function: {e}"))?;

    // Call _start. This is the point where the guest runs.
    // If it exceeds the timeout, the epoch watchdog will cause a trap.
    // If it exceeds memory limits, StoreLimits will deny the growth.
    let exec_result = start_func.call(&mut store, ());

    // ── Step 9: Clean up the watchdog thread ─────────────────────────────
    // We don't care if the watchdog has already fired or not.
    let _ = watchdog.join();

    // ── Step 10: Extract captured output ─────────────────────────────────
    // Read the stdout and stderr pipes regardless of execution result.
    // Even on a trap, partial output may have been written.
    //
    // NOTE: We use contents() rather than try_into_inner() because the
    // WasiP1Ctx still holds an Arc clone of the pipe. contents() reads
    // through the shared Arc<Mutex<BytesMut>> without requiring sole ownership.
    let stdout_bytes = stdout_pipe.contents();
    let _stderr_bytes = stderr_pipe.contents();

    // Check execution result.
    match exec_result {
        Ok(()) => {
            let stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
            Ok(stdout)
        }
        Err(trap) => {
            // Format the trap error and propagate it for classification.
            // MUST use Debug {:?} to capture the Trap code (e.g. Interrupt) and not just the backtrace.
            Err(format!("{:?}", trap).into())
        }
    }
}

// ── Error Classification ─────────────────────────────────────────────────────
// Maps Wasmtime errors into our typed feedback envelope. The LLM gets
// structured, actionable error information — not raw stack traces.

/// Classify a Wasmtime execution error into a `GuardrailResult`.
///
/// The classification logic inspects the error string for known patterns:
/// - "epoch deadline" → timeout
/// - "memory" → memory exceeded
/// - Other traps → generic trap
/// - Anything else → runtime error
fn classify_execution_error(error: Box<dyn std::error::Error + Send + Sync>) -> GuardrailResult {
    // Use Debug formatting {:?} to expose the underlying causes and source chain.
    // Display {} often curtails the root Wasmtime trap reason (like "Interrupt").
    let error_string = format!("{:?}", error);
    let error_lower = error_string.to_lowercase();

    // Epoch interruption in Wasmtime produces "wasm trap: interrupt"
    if error_lower.contains("epoch")
        || error_lower.contains("deadline")
        || error_lower.contains("interrupt")
    {
        GuardrailResult::timeout(error_string)
    } else if error_lower.contains("memory") && error_lower.contains("limit") {
        GuardrailResult::memory_exceeded(error_string)
    } else if error_lower.contains("alloc")
        || error_lower.contains("oom")
        || error_lower.contains("rust_oom")
        || error_lower.contains("alloc_error")
    {
        // OOM from Rust's global allocator inside the WASM guest.
        // This fires when StoreLimits denies memory.grow and the Rust
        // allocator calls __rust_alloc_error_handler → abort.
        GuardrailResult::memory_exceeded(error_string)
    } else if error_lower.contains("unreachable")
        || error_lower.contains("stack overflow")
        || error_lower.contains("out of bounds")
        || error_lower.contains("indirect call")
        || error_lower.contains("integer overflow")
        || error_lower.contains("integer divide by zero")
    {
        GuardrailResult::trap(error_string)
    } else if error_lower.contains("memory") {
        GuardrailResult::memory_exceeded(error_string)
    } else {
        GuardrailResult::runtime_error(error_string)
    }
}
