//! Bounded pure upstream inspection. Observed inputs are not asserted backend requirements.
use nexofolio_common::{EndpointId, EnvironmentId};
use nexofolio_contracts::{
    endpoint::{EndpointFacts, FieldLocation, ValueType},
    knowledge::{Confirmation, EndpointKnowledge, InputRef, Link, Relation},
};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize)]
pub struct TraceStep {
    pub endpoint: EndpointId,
    pub input: Option<InputRef>,
    pub source: Option<Link>,
    pub state: String,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct TraceResult {
    pub environment: Option<EnvironmentId>,
    pub available_environments: Vec<EnvironmentId>,
    pub steps: Vec<TraceStep>,
    pub truncated: bool,
    pub visited_endpoints: Vec<EndpointId>,
}
pub struct TraceOptions<'a> {
    pub environment: Option<EnvironmentId>,
    pub known_inputs: &'a [InputRef],
    pub authenticated: bool,
    pub max_depth: usize,
    pub max_steps: usize,
}
pub fn trace_upstream(
    target: EndpointId,
    facts: &[EndpointFacts],
    knowledge: &[(EndpointId, EndpointKnowledge)],
    options: TraceOptions<'_>,
) -> TraceResult {
    let environment = if let Some(fact) = facts.iter().find(|f| f.summary.id == target) {
        let info = knowledge
            .iter()
            .find(|(id, _)| *id == target)
            .map(|(_, k)| k);
        match super::trace_environment(fact, info, options.environment) {
            Ok(environment) => environment,
            Err(available_environments) => {
                return TraceResult {
                    environment: None,
                    available_environments,
                    steps: vec![TraceStep {
                        endpoint: target,
                        input: None,
                        source: None,
                        state: if options.environment.is_some() {
                            "environment_unavailable"
                        } else {
                            "environment_required"
                        }
                        .into(),
                        reason:
                            "请从该接口已有的环境中明确选择，再读取认证和参数来源；不混合不同环境"
                                .into(),
                    }],
                    truncated: false,
                    visited_endpoints: vec![target],
                };
            }
        }
    } else {
        options.environment
    };
    let mut trace = Trace {
        environment,
        known: options.known_inputs,
        authenticated: options.authenticated,
        facts,
        knowledge,
        max_depth: options.max_depth.min(8),
        max_steps: options.max_steps.min(100),
        result: TraceResult {
            environment,
            available_environments: environment.into_iter().collect(),
            steps: vec![],
            truncated: false,
            visited_endpoints: vec![],
        },
        active: HashSet::new(),
        done: HashSet::new(),
    };
    trace.visit(target, 0);
    trace.result
}
struct Trace<'a> {
    environment: Option<EnvironmentId>,
    known: &'a [InputRef],
    authenticated: bool,
    facts: &'a [EndpointFacts],
    knowledge: &'a [(EndpointId, EndpointKnowledge)],
    max_depth: usize,
    max_steps: usize,
    result: TraceResult,
    active: HashSet<EndpointId>,
    done: HashSet<EndpointId>,
}
impl Trace<'_> {
    fn push(
        &mut self,
        endpoint: EndpointId,
        input: Option<InputRef>,
        source: Option<Link>,
        state: &str,
        reason: &str,
    ) {
        if self.result.steps.len() >= self.max_steps {
            self.result.truncated = true;
            return;
        }
        self.result.steps.push(TraceStep {
            endpoint,
            input,
            source,
            state: state.into(),
            reason: reason.into(),
        });
    }
    fn visit(&mut self, endpoint: EndpointId, depth: usize) {
        if depth > self.max_depth {
            self.result.truncated = true;
            return;
        }
        if self.active.contains(&endpoint) {
            self.push(
                endpoint,
                None,
                None,
                "cycle",
                "上游循环，停止展开；需要当前输入或其他发现来源",
            );
            return;
        }
        if !self.done.insert(endpoint) {
            return;
        }
        self.active.insert(endpoint);
        self.result.visited_endpoints.push(endpoint);
        let Some(facts) = self.facts.iter().find(|f| f.summary.id == endpoint) else {
            self.push(
                endpoint,
                None,
                None,
                "knowledge_gap",
                "接口资料未加载，可继续读取该接口",
            );
            return;
        };
        let knowledge = self
            .knowledge
            .iter()
            .find(|(id, _)| *id == endpoint)
            .map(|(_, k)| k);
        if let Some(k) = knowledge {
            let auth = k.note.as_ref().map(|n| &n.context.authentication);
            if auth.is_some_and(|a| {
                a.iter()
                    .any(|usage| usage.source.environment == self.environment)
            }) {
                self.push(
                    endpoint,
                    None,
                    None,
                    if self.authenticated {
                        "authentication_reused"
                    } else {
                        "authentication_gap"
                    },
                    if self.authenticated {
                        "复用调用者声明的当前有效业务认证；不读取历史凭证"
                    } else {
                        "观察到认证使用，可靠登录/凭证来源尚未闭合；应提供当前有效认证或补资料"
                    },
                );
            }
        }
        let environment = self.environment;
        if let Some(k) = knowledge {
            for link in k.links.iter().filter(|l| {
                l.to == endpoint
                    && l.context.environment == environment
                    && matches!(
                        l.relation,
                        Relation::ConfirmedPrerequisite
                            | Relation::ConditionalPrerequisite
                            | Relation::RecommendedCheck
                            | Relation::AuthenticationUse
                    )
            }) {
                match link.relation {
                    Relation::ConfirmedPrerequisite
                        if link.context.confirmation == Confirmation::Confirmed
                            && link.context.condition.is_none() =>
                    {
                        self.push(
                            endpoint,
                            None,
                            Some(link.clone()),
                            "confirmed_prerequisite",
                            "有独立确认依据的前置；仍需检查来源接口自身输入",
                        );
                        self.visit(link.from, depth + 1);
                    }
                    Relation::ConditionalPrerequisite => self.push(
                        endpoint,
                        None,
                        Some(link.clone()),
                        "condition_unresolved",
                        "需要核对关系触发条件；不能默认每次执行",
                    ),
                    Relation::RecommendedCheck => self.push(
                        endpoint,
                        None,
                        Some(link.clone()),
                        "recommended_check",
                        "推荐核对，未证明强制",
                    ),
                    Relation::AuthenticationUse => self.push(
                        endpoint,
                        None,
                        Some(link.clone()),
                        if self.authenticated {
                            "authentication_reused"
                        } else {
                            "authentication_gap"
                        },
                        "认证使用与业务登录状态分开；不返回历史凭证",
                    ),
                    _ => self.push(
                        endpoint,
                        None,
                        Some(link.clone()),
                        "reference",
                        "前置确认状态或触发条件仍需核对，不自动强制",
                    ),
                }
            }
        }
        let inputs: Vec<_> = facts
            .fields
            .iter()
            .filter(|f| {
                !matches!(
                    f.location,
                    FieldLocation::ResponseBody { .. }
                        | nexofolio_contracts::endpoint::FieldLocation::ResponseHeader { .. }
                ) && !f.path.to_string().is_empty()
                    && !f.types.iter().all(|t| {
                        matches!(t, ValueType::Object | ValueType::Array | ValueType::Null)
                    })
            })
            .map(|f| InputRef {
                endpoint,
                location: f.location,
                path: f.path.to_string(),
            })
            .collect();
        for input in inputs {
            if self.result.steps.len() >= self.max_steps {
                self.result.truncated = true;
                break;
            }
            let reusable_source_input =
                self.knowledge
                    .iter()
                    .flat_map(|(_, k)| &k.links)
                    .any(|link| {
                        link.context.source_input.as_ref() == Some(&input)
                            && link.context.environment == self.environment
                            && link.context.confirmation != Confirmation::Candidate
                            && link
                                .context
                                .to_location
                                .zip(link.to_field.as_ref())
                                .is_some_and(|(location, path)| {
                                    self.known.contains(&InputRef {
                                        endpoint: link.to,
                                        location,
                                        path: path.clone(),
                                    })
                                })
                    });
            if self.known.contains(&input) || reusable_source_input {
                self.push(
                    endpoint,
                    Some(input),
                    None,
                    "input_reused",
                    "调用者已提供该输入，不重复取得",
                );
                continue;
            }
            let links: Vec<_> = knowledge
                .into_iter()
                .flat_map(|k| &k.links)
                .filter(|l| {
                    l.to == endpoint
                        && l.to_field.as_deref() == Some(&input.path)
                        && l.context.to_location == Some(input.location)
                        && l.context.environment == self.environment
                })
                .cloned()
                .collect();
            let mut followed = false;
            for link in links {
                if link.relation == Relation::ParameterSource
                    && link.context.confirmation != Confirmation::Candidate
                {
                    self.push(
                        endpoint,
                        Some(input.clone()),
                        Some(link.clone()),
                        "observed_source",
                        "可参考当前样本来源；先检查来源接口自身输入，未证明必调或本次值有效",
                    );
                    self.visit(link.from, depth + 1);
                    followed = true;
                } else {
                    self.push(
                        endpoint,
                        Some(input.clone()),
                        Some(link),
                        "reference",
                        "实体/配置/候选或条件参考，不能自动用作缺失输入的发现步骤",
                    );
                }
            }
            if !followed {
                self.push(
                    endpoint,
                    Some(input),
                    None,
                    "input_gap",
                    "无可用的缺失输入发现来源；询问当前输入或补接口资料。观测出现不证明后台必填",
                );
            }
        }
        self.active.remove(&endpoint);
    }
}
