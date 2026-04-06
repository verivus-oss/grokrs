//! Approval-mode-aware decision resolver for the agent executor.
//!
//! Translates [`Decision`] from the policy engine into a final allow/deny
//! verdict, applying the `approval_mode` setting from [`SessionConfig`]:
//!
//! - `"allow"` — map `Ask` to `Allow` (bypass approval; use with caution).
//! - `"deny"` — map `Ask` to `Deny` (fail-closed; no interactive approval).
//! - `"interactive"` (or any other value) — keep `Ask` as `Ask` (requires
//!   the approval broker from spec 03; treated as denial until implemented).
//!
//! `Allow` and `Deny` decisions from the engine are **never** modified by
//! `approval_mode` — only `Ask` is affected.

use grokrs_policy::Decision;

/// Resolved decision after applying approval_mode logic.
///
/// Unlike [`Decision`] which has three variants, this enum collapses to
/// two: the tool call either proceeds or it does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedDecision {
    /// The effect is permitted; proceed with execution.
    Allow,
    /// The effect is denied; return an error message to the model.
    Deny {
        /// Human-readable reason for the denial.
        reason: String,
    },
}

/// Resolve a [`Decision`] using the configured `approval_mode`.
///
/// - `Allow` always maps to `ResolvedDecision::Allow`.
/// - `Deny` always maps to `ResolvedDecision::Deny`.
/// - `Ask` is resolved based on `approval_mode`:
///   - `"allow"` → `Allow`
///   - `"deny"` → `Deny`
///   - anything else → `Deny` (with "approval required" message)
#[must_use]
pub fn resolve_decision(decision: Decision, approval_mode: &str) -> ResolvedDecision {
    match decision {
        Decision::Allow { .. } => ResolvedDecision::Allow,
        Decision::Deny { reason } => ResolvedDecision::Deny {
            reason: reason.to_owned(),
        },
        Decision::Ask { reason } => match approval_mode {
            "allow" => ResolvedDecision::Allow,
            "deny" => ResolvedDecision::Deny {
                reason: reason.to_owned(),
            },
            // "interactive" or any unrecognised value: approval broker not yet
            // available, so this is effectively a denial with a helpful message.
            _ => ResolvedDecision::Deny {
                reason: format!(
                    "approval required (approval_mode='{approval_mode}'): {reason}. \
                     Set approval_mode = 'allow' in [session] config to bypass, \
                     or wait for the approval broker (spec 03)."
                ),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_decision_passes_through_all_modes() {
        let decision = Decision::Allow {
            reason: "workspace reads are allowed",
        };
        for mode in &["allow", "deny", "interactive", "unknown"] {
            assert_eq!(
                resolve_decision(decision.clone(), mode),
                ResolvedDecision::Allow,
                "Allow should pass through for approval_mode={mode}"
            );
        }
    }

    #[test]
    fn deny_decision_passes_through_all_modes() {
        let decision = Decision::Deny {
            reason: "network access is denied by default",
        };
        for mode in &["allow", "deny", "interactive", "unknown"] {
            let resolved = resolve_decision(decision.clone(), mode);
            assert!(
                matches!(resolved, ResolvedDecision::Deny { .. }),
                "Deny should pass through for approval_mode={mode}"
            );
        }
    }

    #[test]
    fn ask_with_allow_mode_permits() {
        let decision = Decision::Ask {
            reason: "network access requires explicit approval flow",
        };
        assert_eq!(resolve_decision(decision, "allow"), ResolvedDecision::Allow);
    }

    #[test]
    fn ask_with_deny_mode_denies() {
        let decision = Decision::Ask {
            reason: "network access requires explicit approval flow",
        };
        let resolved = resolve_decision(decision, "deny");
        match resolved {
            ResolvedDecision::Deny { reason } => {
                assert!(reason.contains("network access requires explicit approval flow"));
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[test]
    fn ask_with_interactive_mode_denies_with_helpful_message() {
        let decision = Decision::Ask {
            reason: "shell spawn requires explicit approval flow",
        };
        let resolved = resolve_decision(decision, "interactive");
        match resolved {
            ResolvedDecision::Deny { reason } => {
                assert!(reason.contains("approval required"));
                assert!(reason.contains("interactive"));
                assert!(reason.contains("shell spawn"));
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[test]
    fn ask_with_unknown_mode_denies() {
        let decision = Decision::Ask {
            reason: "some effect",
        };
        let resolved = resolve_decision(decision, "gibberish");
        assert!(matches!(resolved, ResolvedDecision::Deny { .. }));
    }
}
