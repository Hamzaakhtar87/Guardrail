// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// swarm.rs — Swarm Parallelization (Phase 5)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Asynchronous orchestrator that scales the Scout agents into a concurrent
// execution paradigm. Uses token buckets (Semaphores) to actively throttle
// LLM requests, saturated to the physical maximum of the host memory.

use crate::scout::{self, CandidateEvaluation};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

/// Maximum number of concurrent LLM evaluations.
/// Set to 2 to perfectly saturate M1 Unified Memory without risking OOM panics.
const MAX_CONCURRENT_LLM_REQUESTS: usize = 2;

/// Orchestrates a cohort of automated targets through the Scout LLM agent.
/// Blocks until all targets in the cohort have been evaluated.
pub async fn evaluate_cohort(targets: Vec<&str>) -> Vec<(String, Result<CandidateEvaluation, String>)> {
    println!("  [SWARM] Initializing concurrent evaluation for {} targets...", targets.len());
    println!("  [SWARM] Throttle active: Max {} concurrent LLM contexts.", MAX_CONCURRENT_LLM_REQUESTS);

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_LLM_REQUESTS));
    let mut tasks = JoinSet::new();

    for target_url in targets {
        let target_string = target_url.to_string();
        let sem_clone = Arc::clone(&semaphore);
        
        tasks.spawn(async move {
            // Wait strictly for a permit before initializing the LLM context.
            let _permit = sem_clone
                .acquire()
                .await
                .expect("Critical Failure: Thread semaphore closed unexpectedly.");

            let result = scout::evaluate_candidate(&target_string).await;
            (target_string, result)
        });
    }

    let mut cohort_results = Vec::new();
    while let Some(res) = tasks.join_next().await {
        match res {
            Ok(evaluation_payload) => cohort_results.push(evaluation_payload),
            Err(e) => {
                println!("  [SWARM FATAL] Concurrent task panicked: {}", e);
            }
        }
    }

    cohort_results
}
