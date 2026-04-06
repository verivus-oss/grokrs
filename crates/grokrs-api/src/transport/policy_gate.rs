/// The result of a policy evaluation for an outbound network request.
///
/// This mirrors the decision model in `grokrs-policy` but is defined here
/// so that `grokrs-api` does not depend on `grokrs-policy` at compile time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    /// The request is allowed to proceed.
    Allow,
    /// The request is denied with a reason.
    Deny { reason: String },
    /// The request requires interactive approval before proceeding.
    Ask,
}

/// Trait for policy evaluation before outbound HTTP requests.
///
/// Implementations are injected by the caller. `grokrs-api` does not depend
/// on `grokrs-policy` at compile time. The integration layer (U23) will
/// provide an implementation that bridges `PolicyEngine` into this trait.
pub trait PolicyGate: Send + Sync {
    /// Evaluate whether an outbound connection to the given host is permitted.
    fn evaluate_network(&self, host: &str) -> PolicyDecision;
}

/// A policy gate that allows all requests unconditionally.
///
/// This is useful for testing and for scenarios where policy is managed
/// at a higher layer.
#[derive(Debug, Clone, Copy)]
pub struct AllowAllGate;

impl PolicyGate for AllowAllGate {
    fn evaluate_network(&self, _host: &str) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_all_gate_allows_everything() {
        let gate = AllowAllGate;
        assert_eq!(gate.evaluate_network("api.x.ai"), PolicyDecision::Allow);
        assert_eq!(
            gate.evaluate_network("evil.example.com"),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn policy_decision_deny_carries_reason() {
        let decision = PolicyDecision::Deny {
            reason: "network access is denied by default".into(),
        };
        match decision {
            PolicyDecision::Deny { reason } => {
                assert_eq!(reason, "network access is denied by default");
            }
            _ => panic!("expected Deny"),
        }
    }
}
