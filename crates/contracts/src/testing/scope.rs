use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use nexofolio_common::{EnvironmentId, ProjectId};

use crate::scope::{
    CollectTarget, EnvironmentSelector, ResolvedTarget, ScopeError, SiteBinding, SiteRegistry,
    SiteScope, TargetResolver,
};

/// Seeds projects and environments so conformance suites can run against any implementation.
#[async_trait]
pub trait ScopeFixture: Send + Sync {
    async fn create_project(&self) -> ProjectId;
    async fn create_environment(&self, project: ProjectId, name: &str) -> EnvironmentId;
    async fn environment_names(&self, project: ProjectId) -> Vec<String>;
}

#[derive(Default)]
struct State {
    environments: HashMap<ProjectId, Vec<(EnvironmentId, String)>>,
    sites: Vec<SiteBinding>,
    annotations: Vec<(EnvironmentId, SiteScope)>,
}

/// Fake access: projects, environments, the site registry and site annotations in memory.
#[derive(Default)]
pub struct InMemoryScope {
    state: Mutex<State>,
}

impl InMemoryScope {
    pub fn new() -> Self {
        Self::default()
    }

    /// Site annotations recorded by [`TargetResolver::resolve`] for one environment.
    pub fn site_annotations(&self, environment: EnvironmentId) -> Vec<SiteScope> {
        let state = self.state.lock().expect("fake lock");
        state
            .annotations
            .iter()
            .filter(|(env, _)| *env == environment)
            .map(|(_, site)| site.clone())
            .collect()
    }

    fn owns(
        state: &State,
        project: ProjectId,
        environment: EnvironmentId,
    ) -> Result<(), ScopeError> {
        let envs = state
            .environments
            .get(&project)
            .ok_or(ScopeError::UnknownProject)?;
        if envs.iter().any(|(id, _)| *id == environment) {
            Ok(())
        } else {
            Err(ScopeError::UnknownEnvironment)
        }
    }
}

/// Same rule as access: exact names, 1..=64 chars, no surrounding whitespace or controls.
fn valid_environment_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= 64
        && name.trim() == name
        && !name.chars().any(char::is_control)
}

#[async_trait]
impl ScopeFixture for InMemoryScope {
    async fn create_project(&self) -> ProjectId {
        let id = ProjectId::new();
        self.state
            .lock()
            .expect("fake lock")
            .environments
            .insert(id, Vec::new());
        id
    }

    async fn create_environment(&self, project: ProjectId, name: &str) -> EnvironmentId {
        let id = EnvironmentId::new();
        let mut state = self.state.lock().expect("fake lock");
        state
            .environments
            .entry(project)
            .or_default()
            .push((id, name.to_owned()));
        id
    }

    async fn environment_names(&self, project: ProjectId) -> Vec<String> {
        let state = self.state.lock().expect("fake lock");
        state
            .environments
            .get(&project)
            .map(|envs| envs.iter().map(|(_, name)| name.clone()).collect())
            .unwrap_or_default()
    }
}

#[async_trait]
impl TargetResolver for InMemoryScope {
    async fn resolve(&self, target: &CollectTarget) -> Result<ResolvedTarget, ScopeError> {
        let mut state = self.state.lock().expect("fake lock");
        let envs = state
            .environments
            .get_mut(&target.project_id)
            .ok_or(ScopeError::UnknownProject)?;
        let environment_id = match &target.environment {
            None => None,
            Some(EnvironmentSelector::Id(id)) => {
                if !envs.iter().any(|(env, _)| env == id) {
                    return Err(ScopeError::UnknownEnvironment);
                }
                Some(*id)
            }
            Some(EnvironmentSelector::Name(name)) => {
                if !valid_environment_name(name) {
                    return Err(ScopeError::InvalidEnvironmentName(name.clone()));
                }
                match envs.iter().find(|(_, existing)| existing == name) {
                    Some((id, _)) => Some(*id),
                    None => {
                        let id = EnvironmentId::new();
                        envs.push((id, name.clone()));
                        Some(id)
                    }
                }
            }
        };
        if let (Some(env), Some(site)) = (environment_id, &target.site) {
            let annotation = (env, site.clone());
            if !state.annotations.contains(&annotation) {
                state.annotations.push(annotation);
            }
        }
        Ok(ResolvedTarget {
            project_id: target.project_id,
            environment_id,
            site: target.site.clone(),
            source_url: target.source_url.clone(),
        })
    }
}

#[async_trait]
impl SiteRegistry for InMemoryScope {
    async fn lookup(&self, page_url: &str) -> Result<Option<SiteBinding>, ScopeError> {
        let state = self.state.lock().expect("fake lock");
        Ok(state
            .sites
            .iter()
            .filter(|binding| binding.site.contains(page_url))
            .max_by_key(|binding| binding.site.specificity())
            .cloned())
    }

    async fn bind(&self, binding: SiteBinding) -> Result<(), ScopeError> {
        let mut state = self.state.lock().expect("fake lock");
        Self::owns(&state, binding.project_id, binding.environment_id)?;
        state.sites.retain(|existing| existing.site != binding.site);
        state.sites.push(binding);
        Ok(())
    }
}

fn target(project_id: ProjectId, environment: Option<EnvironmentSelector>) -> CollectTarget {
    CollectTarget {
        project_id,
        environment,
        site: None,
        source_url: None,
    }
}

/// Behaviour every [`TargetResolver`] must show. Panics on the first violation.
pub async fn target_resolver_conformance<R: TargetResolver + ScopeFixture>(resolver: &R) {
    let project = resolver.create_project().await;
    let other = resolver.create_project().await;
    let prod = resolver.create_environment(project, "正式环境").await;
    let foreign = resolver.create_environment(other, "正式环境").await;

    let resolved = resolver
        .resolve(&target(project, Some(EnvironmentSelector::Id(prod))))
        .await
        .expect("own environment resolves");
    assert_eq!(resolved.environment_id, Some(prod));

    assert_eq!(
        resolver.resolve(&target(ProjectId::new(), None)).await,
        Err(ScopeError::UnknownProject),
        "unknown project"
    );
    assert_eq!(
        resolver
            .resolve(&target(project, Some(EnvironmentSelector::Id(foreign))))
            .await,
        Err(ScopeError::UnknownEnvironment),
        "environment of another project"
    );

    let by_name = || target(project, Some(EnvironmentSelector::Name("测试环境".into())));
    let created = resolver
        .resolve(&by_name())
        .await
        .expect("name creates")
        .environment_id;
    let again = resolver
        .resolve(&by_name())
        .await
        .expect("name reuses")
        .environment_id;
    assert!(created.is_some());
    assert_eq!(created, again, "same name resolves to the same environment");
    let names = resolver.environment_names(project).await;
    assert_eq!(names.iter().filter(|name| *name == "测试环境").count(), 1);

    let existing = resolver
        .resolve(&target(
            project,
            Some(EnvironmentSelector::Name("正式环境".into())),
        ))
        .await
        .expect("existing name");
    assert_eq!(existing.environment_id, Some(prod));

    assert!(
        matches!(
            resolver
                .resolve(&target(
                    project,
                    Some(EnvironmentSelector::Name(" 正式环境".into()))
                ))
                .await,
            Err(ScopeError::InvalidEnvironmentName(_))
        ),
        "names are exact, not trimmed"
    );

    let declarations = resolver
        .resolve(&target(project, None))
        .await
        .expect("no environment");
    assert_eq!(declarations.environment_id, None);

    let mut with_site = target(project, Some(EnvironmentSelector::Id(prod)));
    with_site.site = Some(SiteScope::new("https://shop.example.com", "/").expect("site"));
    with_site.source_url = Some("https://shop.example.com/v3/api-docs".into());
    let first = resolver.resolve(&with_site).await.expect("site annotation");
    let second = resolver
        .resolve(&with_site)
        .await
        .expect("repeat is harmless");
    assert_eq!(first, second);
    assert_eq!(first.site, with_site.site);
    assert_eq!(first.source_url, with_site.source_url);
}

/// Behaviour every [`SiteRegistry`] must show. Panics on the first violation.
pub async fn site_registry_conformance<R: SiteRegistry + ScopeFixture>(registry: &R) {
    let project = registry.create_project().await;
    let prod = registry.create_environment(project, "正式环境").await;
    let test = registry.create_environment(project, "测试环境").await;
    let other = registry.create_project().await;
    let foreign = registry.create_environment(other, "正式环境").await;

    let host = format!("https://{}.example.test", project);
    let site = |prefix: &str| SiteScope::new(&host, prefix).expect("site");
    let binding = |prefix: &str, environment_id| SiteBinding {
        site: site(prefix),
        project_id: project,
        environment_id,
    };

    assert_eq!(
        registry.lookup(&format!("{host}/")).await,
        Ok(None),
        "empty registry"
    );

    registry.bind(binding("/", prod)).await.expect("bind root");
    registry
        .bind(binding("/test", test))
        .await
        .expect("bind prefix");

    let found = |url: String| async move { registry.lookup(&url).await.expect("lookup") };
    assert_eq!(
        found(format!("{host}/#/order")).await,
        Some(binding("/", prod))
    );
    assert_eq!(
        found(format!("{host}/test/orders?x=1")).await,
        Some(binding("/test", test)),
        "longest prefix wins"
    );
    assert_eq!(
        found(format!("{host}/testing")).await,
        Some(binding("/", prod)),
        "whole segments only"
    );
    assert_eq!(
        found(format!("{}/", host.to_uppercase())).await,
        Some(binding("/", prod)),
        "origin is case-insensitive"
    );
    assert_eq!(found("https://unrelated.example.test/".into()).await, None);

    registry.bind(binding("/", test)).await.expect("rebind");
    assert_eq!(
        found(format!("{host}/")).await,
        Some(binding("/", test)),
        "one scope, one binding"
    );

    assert_eq!(
        registry.bind(binding("/other", foreign)).await,
        Err(ScopeError::UnknownEnvironment),
        "environment of another project"
    );
    assert_eq!(
        registry
            .bind(SiteBinding {
                site: site("/x"),
                project_id: ProjectId::new(),
                environment_id: prod
            })
            .await,
        Err(ScopeError::UnknownProject)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_passes_target_resolver_conformance() {
        target_resolver_conformance(&InMemoryScope::new()).await;
    }

    #[tokio::test]
    async fn fake_passes_site_registry_conformance() {
        site_registry_conformance(&InMemoryScope::new()).await;
    }

    #[tokio::test]
    async fn fake_records_site_annotations_once() {
        let scope = InMemoryScope::new();
        let project = scope.create_project().await;
        let env = scope.create_environment(project, "正式环境").await;
        let mut target = target(project, Some(EnvironmentSelector::Id(env)));
        target.site = Some(SiteScope::new("https://shop.example.com", "/").unwrap());
        scope.resolve(&target).await.unwrap();
        scope.resolve(&target).await.unwrap();
        assert_eq!(scope.site_annotations(env), vec![target.site.unwrap()]);
    }
}
