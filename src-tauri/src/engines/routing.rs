use std::collections::HashSet;
use std::fmt;

use crate::domain::EngineLocation;

use super::providers::{EngineDescriptor, EngineRole};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyMode {
    /// Audio and transcript text must remain on this device. Cloud engines are
    /// neither valid primaries nor eligible fallbacks.
    LocalOnly,
    /// The user has explicitly allowed cloud processing for this route.
    CloudAllowed,
}

/// An ordered route built only from trusted adapter-derived descriptors.
///
/// Descriptor fields and constructors are private, so public callers cannot
/// set `Local` on an arbitrary provider. The remaining P2 boundary is runtime
/// enforcement: Rust's type system cannot prove that an audited, in-process
/// decoder never opens a socket. Production must keep the local adapter
/// registry restricted to reviewed crate implementations; an OS-level network
/// sandbox would be a separate defense-in-depth milestone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePlan {
    candidates: Vec<EngineDescriptor>,
    blocked_cloud_fallbacks: Vec<EngineDescriptor>,
}

impl RoutePlan {
    pub fn build(
        primary: EngineDescriptor,
        fallbacks: impl IntoIterator<Item = EngineDescriptor>,
        privacy: PrivacyMode,
    ) -> Result<Self, RoutingError> {
        if privacy == PrivacyMode::LocalOnly && primary.location() == EngineLocation::Cloud {
            return Err(RoutingError::CloudPrimaryBlocked(primary.id().into()));
        }

        let mut seen = HashSet::from([primary.id().to_owned()]);
        let mut candidates = vec![primary];
        let mut blocked_cloud_fallbacks = Vec::new();

        for fallback in fallbacks {
            if fallback.role() != candidates[0].role() {
                return Err(RoutingError::RoleMismatch {
                    primary: candidates[0].role(),
                    fallback: fallback.role(),
                });
            }
            if !seen.insert(fallback.id().to_owned()) {
                continue;
            }
            if privacy == PrivacyMode::LocalOnly && fallback.location() == EngineLocation::Cloud {
                blocked_cloud_fallbacks.push(fallback);
            } else {
                candidates.push(fallback);
            }
        }

        debug_assert!(
            privacy != PrivacyMode::LocalOnly
                || candidates
                    .iter()
                    .all(|candidate| candidate.location() == EngineLocation::Local)
        );

        Ok(Self {
            candidates,
            blocked_cloud_fallbacks,
        })
    }

    pub fn candidates(&self) -> &[EngineDescriptor] {
        &self.candidates
    }

    /// Exposed for diagnostics so the UI can explain why a configured fallback
    /// was skipped without ever handing it to the execution loop.
    pub fn blocked_cloud_fallbacks(&self) -> &[EngineDescriptor] {
        &self.blocked_cloud_fallbacks
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingError {
    CloudPrimaryBlocked(String),
    RoleMismatch {
        primary: EngineRole,
        fallback: EngineRole,
    },
}

impl fmt::Display for RoutingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CloudPrimaryBlocked(engine) => {
                write!(
                    formatter,
                    "cloud engine {engine} is blocked by local-only mode"
                )
            }
            Self::RoleMismatch { primary, fallback } => write!(
                formatter,
                "cannot route from a {primary:?} engine to a {fallback:?} engine"
            ),
        }
    }
}

impl std::error::Error for RoutingError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(id: &str) -> EngineDescriptor {
        EngineDescriptor::test_local(id, EngineRole::Transcription)
    }

    fn cloud(id: &str) -> EngineDescriptor {
        EngineDescriptor::test_cloud(id, EngineRole::Transcription)
    }

    #[test]
    fn local_only_plan_never_exposes_a_cloud_fallback() {
        let local_primary = local("local-whisper");
        let local_backup = local("local-whisper-backup");
        let cloud_fallback = cloud("cloud-speech");
        let plan = RoutePlan::build(
            local_primary.clone(),
            [
                cloud_fallback.clone(),
                local_backup.clone(),
                cloud_fallback.clone(),
            ],
            PrivacyMode::LocalOnly,
        )
        .unwrap();

        assert_eq!(plan.candidates(), &[local_primary, local_backup]);
        assert_eq!(plan.blocked_cloud_fallbacks(), &[cloud_fallback]);
        assert!(plan
            .candidates()
            .iter()
            .all(|engine| engine.location() == EngineLocation::Local));
    }

    #[test]
    fn local_only_rejects_an_explicit_cloud_primary() {
        assert_eq!(
            RoutePlan::build(
                cloud("cloud-speech"),
                [local("local-whisper")],
                PrivacyMode::LocalOnly
            ),
            Err(RoutingError::CloudPrimaryBlocked("cloud-speech".into()))
        );
    }

    #[test]
    fn cloud_allowed_plan_preserves_explicit_order() {
        let local = local("local-whisper");
        let cloud = cloud("cloud-speech");
        let plan =
            RoutePlan::build(local.clone(), [cloud.clone()], PrivacyMode::CloudAllowed).unwrap();
        assert_eq!(plan.candidates(), &[local, cloud]);
        assert!(plan.blocked_cloud_fallbacks().is_empty());
    }

    #[test]
    fn routes_cannot_cross_engine_roles() {
        let cleanup = EngineDescriptor::test_local("cleanup", EngineRole::Cleanup);
        assert_eq!(
            RoutePlan::build(local("local-whisper"), [cleanup], PrivacyMode::CloudAllowed),
            Err(RoutingError::RoleMismatch {
                primary: EngineRole::Transcription,
                fallback: EngineRole::Cleanup,
            })
        );
    }
}
