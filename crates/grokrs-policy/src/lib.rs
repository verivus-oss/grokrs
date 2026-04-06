use grokrs_cap::WorkspacePath;
use grokrs_core::PolicyConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    FsRead(WorkspacePath),
    FsWrite(WorkspacePath),
    ProcessSpawn { program: String },
    NetworkConnect { host: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow { reason: &'static str },
    Ask { reason: &'static str },
    Deny { reason: &'static str },
}

#[derive(Debug, Clone)]
pub struct PolicyEngine {
    config: PolicyConfig,
}

impl PolicyEngine {
    pub fn new(config: PolicyConfig) -> Self {
        Self { config }
    }

    pub fn evaluate(&self, effect: &Effect) -> Decision {
        match effect {
            Effect::FsRead(_) => Decision::Allow {
                reason: "workspace reads are allowed",
            },
            Effect::FsWrite(_) if self.config.allow_workspace_writes => Decision::Allow {
                reason: "workspace writes are allowed by config",
            },
            Effect::FsWrite(_) => Decision::Deny {
                reason: "workspace writes are disabled by config",
            },
            Effect::ProcessSpawn { .. } if self.config.allow_shell => Decision::Ask {
                reason: "shell spawn requires explicit approval flow",
            },
            Effect::ProcessSpawn { .. } => Decision::Deny {
                reason: "shell spawn is denied by default",
            },
            Effect::NetworkConnect { .. } if self.config.allow_network => Decision::Ask {
                reason: "network access requires explicit approval flow",
            },
            Effect::NetworkConnect { .. } => Decision::Deny {
                reason: "network access is denied by default",
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Decision, Effect, PolicyEngine};
    use grokrs_cap::WorkspacePath;
    use grokrs_core::PolicyConfig;

    fn sample_policy() -> PolicyEngine {
        PolicyEngine::new(PolicyConfig {
            allow_network: false,
            allow_shell: false,
            allow_workspace_writes: true,
            max_patch_bytes: 1024,
        })
    }

    #[test]
    fn denies_network_by_default() {
        let decision = sample_policy().evaluate(&Effect::NetworkConnect {
            host: "api.x.ai".into(),
        });
        assert_eq!(
            decision,
            Decision::Deny {
                reason: "network access is denied by default"
            }
        );
    }

    #[test]
    fn allows_workspace_write_when_enabled() {
        let effect = Effect::FsWrite(WorkspacePath::new("docs/specs/00_SPEC.md").unwrap());
        let decision = sample_policy().evaluate(&effect);
        assert_eq!(
            decision,
            Decision::Allow {
                reason: "workspace writes are allowed by config"
            }
        );
    }
}
