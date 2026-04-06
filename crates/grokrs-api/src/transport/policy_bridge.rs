//! Bridges an external policy evaluator into the [`PolicyGate`] trait.
//!
//! `grokrs-api` must not depend on `grokrs-policy` at compile time. This module
//! provides adapter types that let the CLI (or any integration layer) inject a
//! policy evaluation function without introducing a direct dependency.
//!
//! # Typical wiring (in the CLI)
//!
//! ```ignore
//! use grokrs_api::transport::policy_bridge::FnPolicyGate;
//! use grokrs_policy::{Decision, Effect, PolicyEngine};
//!
//! let engine = PolicyEngine::new(config.policy.clone());
//! let gate = FnPolicyGate::new(move |host: &str| {
//!     match engine.evaluate(&Effect::NetworkConnect { host: host.to_owned() }) {
//!         Decision::Allow { .. } => PolicyDecision::Allow,
//!         Decision::Ask { .. }   => PolicyDecision::Ask,
//!         Decision::Deny { reason } => PolicyDecision::Deny { reason },
//!     }
//! });
//! ```

use crate::transport::policy_gate::{PolicyDecision, PolicyGate};

/// A [`PolicyGate`] backed by a caller-supplied function.
///
/// This is the primary integration point between `grokrs-policy` and
/// `grokrs-api`. The CLI constructs a closure that captures a
/// `PolicyEngine` and maps its `Decision` to `PolicyDecision`, then wraps
/// it in this struct. Because the closure is opaque, no compile-time
/// dependency on `grokrs-policy` is required here.
pub struct FnPolicyGate<F>
where
    F: Fn(&str) -> PolicyDecision + Send + Sync,
{
    evaluate_fn: F,
}

impl<F> FnPolicyGate<F>
where
    F: Fn(&str) -> PolicyDecision + Send + Sync,
{
    /// Wrap `f` as a [`PolicyGate`]. The function receives the target host
    /// and must return the appropriate [`PolicyDecision`].
    pub fn new(f: F) -> Self {
        Self { evaluate_fn: f }
    }
}

impl<F> PolicyGate for FnPolicyGate<F>
where
    F: Fn(&str) -> PolicyDecision + Send + Sync,
{
    fn evaluate_network(&self, host: &str) -> PolicyDecision {
        (self.evaluate_fn)(host)
    }
}

impl<F> std::fmt::Debug for FnPolicyGate<F>
where
    F: Fn(&str) -> PolicyDecision + Send + Sync,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FnPolicyGate")
            .field("evaluate_fn", &"<closure>")
            .finish()
    }
}

/// A [`PolicyGate`] that unconditionally denies all outbound network requests.
///
/// Useful as a fail-closed default in tests or when the policy engine is
/// unavailable and the caller wants to guarantee no network access.
#[derive(Debug, Clone, Copy)]
pub struct DenyAllGate;

impl PolicyGate for DenyAllGate {
    fn evaluate_network(&self, host: &str) -> PolicyDecision {
        PolicyDecision::Deny {
            reason: format!("all outbound network requests are denied (host: {host})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fn_policy_gate_delegates_to_closure_allow() {
        let gate = FnPolicyGate::new(|_host| PolicyDecision::Allow);
        assert_eq!(gate.evaluate_network("api.x.ai"), PolicyDecision::Allow);
    }

    #[test]
    fn fn_policy_gate_delegates_to_closure_deny() {
        let gate = FnPolicyGate::new(|host| PolicyDecision::Deny {
            reason: format!("blocked: {host}"),
        });
        let result = gate.evaluate_network("evil.example.com");
        match result {
            PolicyDecision::Deny { reason } => {
                assert_eq!(reason, "blocked: evil.example.com");
            }
            other => panic!("expected Deny, got: {other:?}"),
        }
    }

    #[test]
    fn fn_policy_gate_delegates_to_closure_ask() {
        let gate = FnPolicyGate::new(|_host| PolicyDecision::Ask);
        assert_eq!(gate.evaluate_network("api.x.ai"), PolicyDecision::Ask);
    }

    #[test]
    fn fn_policy_gate_receives_correct_host() {
        let gate = FnPolicyGate::new(|host| {
            if host == "allowed.example.com" {
                PolicyDecision::Allow
            } else {
                PolicyDecision::Deny {
                    reason: format!("unexpected host: {host}"),
                }
            }
        });
        assert_eq!(
            gate.evaluate_network("allowed.example.com"),
            PolicyDecision::Allow
        );
        assert!(matches!(
            gate.evaluate_network("other.example.com"),
            PolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn deny_all_gate_denies_everything() {
        let gate = DenyAllGate;
        for host in &["api.x.ai", "localhost", "evil.example.com", ""] {
            match gate.evaluate_network(host) {
                PolicyDecision::Deny { reason } => {
                    assert!(
                        reason.contains(host),
                        "deny reason should contain the host '{host}', got: {reason}"
                    );
                    assert!(reason.contains("denied"));
                }
                other => panic!("expected Deny for host '{host}', got: {other:?}"),
            }
        }
    }

    #[test]
    fn deny_all_gate_is_debug() {
        let gate = DenyAllGate;
        let debug = format!("{gate:?}");
        assert_eq!(debug, "DenyAllGate");
    }

    #[test]
    fn fn_policy_gate_is_debug() {
        let gate = FnPolicyGate::new(|_| PolicyDecision::Allow);
        let debug = format!("{gate:?}");
        assert!(debug.contains("FnPolicyGate"));
        assert!(debug.contains("<closure>"));
    }

    #[test]
    fn fn_policy_gate_with_stateful_closure() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();
        let gate = FnPolicyGate::new(move |_host| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
            PolicyDecision::Allow
        });

        gate.evaluate_network("a.example.com");
        gate.evaluate_network("b.example.com");
        gate.evaluate_network("c.example.com");
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }
}
