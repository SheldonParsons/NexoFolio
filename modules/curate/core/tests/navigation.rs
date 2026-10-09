#[path = "stage1/legacy_samples.rs"]
mod legacy_samples;
mod stage1;
use async_trait::async_trait;
use nexofolio_contracts::{endpoint::FieldLocation, knowledge::*};
use nexofolio_curate::*;
use serde_json::json;

#[test]
fn recorded_navigation_has_scoped_directions_and_preserves_discovery_gaps() {
    let f = stage1::fixture();
    let links = navigation_links(&f.facts, &f.signals);
    let config = f.endpoint("commodityModelNonstandardPoint/add").summary.id;
    let option = f
        .endpoint("commodityModelNonstandardPointConfig/listAll")
        .summary
        .id;
    assert!(links.iter().any(|l| {
        l.from == option
            && l.to == config
            && l.to_field.ends_with("configId")
            && l.context
                .sources
                .iter()
                .any(|s| s.pointer.as_deref().is_some_and(|p| p.contains("/19/")))
    }));
    let create = f.endpoint("saveAndSubmitConsultTechnology").summary.id;
    let project = f.endpoint("getProjectInfoByParam").summary.id;
    assert!(links.iter().any(|l| l.from == project
        && l.to == create
        && l.to_field == "opptyProjectId"
        && l.relation == Relation::EntityReference));
    assert!(
        links
            .iter()
            .all(|l| l.relation != Relation::ConfirmedPrerequisite
                && !["msg", "size", "sort", "status"].contains(&l.to_field.as_str()))
    );
    assert!(
        links
            .iter()
            .any(|l| l.to_field.ends_with("taskId") && l.context.matched_fields.len() >= 2)
    );
    assert!(links.iter().any(|l| l.to_field == "sliderCaptchaId"));
    assert!(links.iter().all(|l| {
        [Confirmation::Observed, Confirmation::Direct].contains(&l.context.confirmation)
            && l.context.environment == Some(f.environment())
            && l.context.sources.len() >= 2
    }));
    let baan = f.endpoint("getBaanCommodity").summary.id;
    assert!(links.iter().any(|l| {
        l.from == baan
            && l.to_field.ends_with("code")
            && l.context
                .matched_fields
                .iter()
                .any(|m| m.to.ends_with("unit"))
            && l.context
                .matched_fields
                .iter()
                .any(|m| m.to.ends_with("modelOrName"))
    }));
    assert!(links.iter().any(
        |l| l.relation == Relation::Dictionary && l.evidence.contains("配置不是已验证后台必填")
    ));
    println!(
        "sanitized measured links={} endpoints={} signals={}",
        links.len(),
        f.facts.len(),
        f.signals.len()
    );
}

struct FixedModel;
#[async_trait]
impl Completions for FixedModel {
    async fn complete(&self, prompt: &str) -> Result<String, CompletionError> {
        assert!(prompt.contains("siblings"));
        let json_start =
            prompt.find("字段和可核查的证据：").unwrap() + "字段和可核查的证据：".len();
        let fields: serde_json::Value = serde_json::Deserializer::from_str(&prompt[json_start..])
            .into_iter::<serde_json::Value>()
            .next()
            .unwrap()
            .unwrap();
        Ok(json!({"fields":fields.as_array().unwrap().iter().map(|f| json!({
            "key":f["key"], "display_name":"模型推测名", "description": if f["path"].as_str().unwrap().contains("Sjb") { "设计部回复" } else { "基础含义待确认" },
            "details":[], "unknowns":[]
        })).collect::<Vec<_>>()}).to_string())
    }
}

#[tokio::test]
async fn direct_page_labels_units_envelope_and_unknowns_survive_model_expression() {
    let f = stage1::fixture();
    let facts = f.endpoint("manager/consultTechnology/page");
    let inputs = field_inputs(
        facts,
        &f.signals,
        &f.facts
            .iter()
            .map(|f| f.summary.clone())
            .collect::<Vec<_>>(),
    );
    let pass = Describe::new(FixedModel, "fixed-fixture".into());
    let commands = pass
        .fields(facts, &EndpointKnowledge::default(), &inputs)
        .await
        .unwrap();
    let context = |path: &str| {
        commands
            .iter()
            .find_map(|c| match c {
                Command::PutFieldNote { context, .. }
                    if context.field_path.as_deref() == Some(path) =>
                {
                    Some(context)
                }
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(
        context("data.records[].itemCode").display_name.as_deref(),
        Some("咨询子单号")
    );
    assert_eq!(
        context("data.records[].estimatedAmount")
            .meaning
            .unit
            .as_deref(),
        Some("万")
    );
    assert!(
        context("data.records[].estimatedAmount")
            .meaning
            .unknowns
            .iter()
            .any(|s| s.contains("币种"))
    );
    assert!(
        context("data.records[].status")
            .evidence
            .iter()
            .all(|e| e
                .options
                .iter()
                .all(|o| !["待审批", "审批中", "已通过", "已驳回"]
                    .contains(&o.label.as_deref().unwrap_or(""))))
    );
    let summaries: Vec<_> = f.facts.iter().map(|f| f.summary.clone()).collect();
    let other = f.endpoint("commodityModelNonstandardPoint/add");
    let inputs = field_inputs(other, &f.signals, &[]);
    let commands = pass
        .fields(other, &EndpointKnowledge::default(), &inputs)
        .await
        .unwrap();
    assert!(commands.iter().any(|c| matches!(c,Command::PutFieldNote {text, context, ..} if context.field_path.as_deref()==Some("code") && text.contains("响应码"))));
    let detail = f.endpoint("viewTechnologicalConsult/{id}");
    let commands = pass
        .fields(
            detail,
            &EndpointKnowledge::default(),
            &field_inputs(detail, &f.signals, &[]),
        )
        .await
        .unwrap();
    assert!(
        commands
            .iter()
            .all(|c| !serde_json::to_string(c).unwrap().contains("设计部"))
    );
    // A different page with the same English key cannot inherit consultation labels.
    let mut unrelated = facts.clone();
    unrelated.summary.path_template = "/warehouse/items".into();
    unrelated.summary.id = nexofolio_common::EndpointId::new();
    let mut signals = f.signals.clone();
    signals.extend(
        f.signals
            .iter()
            .filter(|s| s.endpoint == facts.summary.id)
            .cloned()
            .map(|mut s| {
                s.endpoint = unrelated.summary.id;
                s.source.page = Some("unrelated-page".into());
                s
            }),
    );
    assert!(
        field_inputs(&unrelated, &signals, &summaries)
            .iter()
            .all(|i| {
                i.evidence
                    .iter()
                    .all(|e| e.kind != EvidenceKind::DirectSource)
            })
    );
}

#[test]
fn existing_inputs_authentication_and_reference_only_id_stop_unnecessary_queries() {
    let f = stage1::fixture();
    let create = f.endpoint("saveAndSubmitConsultTechnology").summary.id;
    let links = navigation_links(&f.facts, &f.signals);
    let link_rows: Vec<Link> = links
        .iter()
        .map(|p| Link {
            id: nexofolio_common::LinkId::new(),
            from: p.from,
            to: p.to,
            from_field: Some(p.from_field.clone()),
            to_field: Some(p.to_field.clone()),
            relation: p.relation,
            evidence: p.evidence.clone(),
            context: p.context.clone(),
            author: Author::Curate {
                pass: "navigation".into(),
                model: None,
            },
            at: "2026-10-07T00:00:00Z".parse().unwrap(),
        })
        .collect();
    let auth: Vec<AuthenticationUse> = f
        .authentication
        .iter()
        .filter(|(id, _)| *id == create)
        .flat_map(|(_, a)| a.clone())
        .collect();
    let knowledge: Vec<_> = f
        .facts
        .iter()
        .map(|facts| {
            (
                facts.summary.id,
                EndpointKnowledge {
                    note: Some(EndpointNote {
                        endpoint: facts.summary.id,
                        name: None,
                        purpose: None,
                        context: EndpointContext {
                            authentication: if facts.summary.id == create {
                                auth.clone()
                            } else {
                                vec![]
                            },
                            ..Default::default()
                        },
                        author: Author::Curate {
                            pass: "fixture".into(),
                            model: None,
                        },
                        at: "2026-10-07T00:00:00Z".parse().unwrap(),
                    }),
                    links: link_rows
                        .iter()
                        .filter(|l| l.to == facts.summary.id)
                        .cloned()
                        .collect(),
                    ..Default::default()
                },
            )
        })
        .collect();
    let no_auth = trace_upstream(
        create,
        &f.facts,
        &knowledge,
        nexofolio_curate::TraceOptions {
            environment: Some(f.environment()),
            known_inputs: &[],
            authenticated: false,
            max_depth: 4,
            max_steps: 100,
        },
    );
    assert!(
        no_auth
            .steps
            .iter()
            .any(|s| s.state == "authentication_gap")
    );
    assert!(
        no_auth.steps.iter().any(|s| s.state == "reference"
            && s.input.as_ref().is_some_and(|i| i.path == "opptyProjectId"))
    );
    let known = vec![InputRef {
        endpoint: create,
        location: FieldLocation::RequestBody,
        path: "opptyProjectId".into(),
    }];
    let yes_auth = trace_upstream(
        create,
        &f.facts,
        &knowledge,
        nexofolio_curate::TraceOptions {
            environment: Some(f.environment()),
            known_inputs: &known,
            authenticated: true,
            max_depth: 4,
            max_steps: 100,
        },
    );
    assert!(
        yes_auth
            .steps
            .iter()
            .any(|s| s.state == "authentication_reused")
    );
    assert!(yes_auth.steps.iter().any(|s| s.state == "input_reused"));
    assert!(yes_auth.steps.iter().all(|s| {
        !(s.state == "observed_source"
            && s.input
                .as_ref()
                .is_some_and(|i| i.endpoint == create && i.path == "opptyProjectId"))
    }));
    let project_source = f.endpoint("getProjectInfoByParam").summary.id;
    assert!(yes_auth.steps.iter().any(|s| {
        s.state == "input_reused"
            && s.input
                .as_ref()
                .is_some_and(|i| i.endpoint == project_source && i.path == "id")
    }));
    let bounded = trace_upstream(
        create,
        &f.facts,
        &knowledge,
        nexofolio_curate::TraceOptions {
            environment: Some(f.environment()),
            known_inputs: &[],
            authenticated: false,
            max_depth: 1,
            max_steps: 2,
        },
    );
    assert!(bounded.truncated);
    assert_eq!(bounded.steps.len(), 2);
}

#[test]
fn original_extraction_omits_twentieth_option_but_current_extraction_keeps_it() {
    let f = stage1::fixture();
    let raw: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/stage1-observations.json")).unwrap();
    let endpoint = f.endpoint("commodityModelNonstandardPointConfig/listAll");
    let observation = &raw
        .as_array()
        .unwrap()
        .iter()
        .find(|r| {
            r["path"]
                .as_str()
                .unwrap()
                .ends_with("commodityModelNonstandardPointConfig/listAll")
        })
        .unwrap()["observation"];
    let old = legacy_samples::sample_fields(endpoint, "comparison", observation);
    let new = sample_fields(endpoint, "comparison", observation);
    assert!(old.iter().all(|s| !s.row.ends_with("/19")));
    assert!(new.iter().any(|s| s.row.ends_with("/19") && s.name == "id"));
    assert!(new.len() > old.len());
    println!(
        "same sanitized config record: extracted scalars before={} after={}",
        old.len(),
        new.len()
    );
    // These fields exist on the frozen extraction too, so the comparison did not alter identities.
    assert!(
        old.iter()
            .all(|s| new.iter().any(|n| n.endpoint == s.endpoint
                && n.environment == s.environment
                && n.location == s.location
                && n.path == s.path
                && n.value == s.value
                && n.row == s.row))
    );
}

#[test]
fn cycles_are_reported_and_evidence_changes_invalidate_only_generated_knowledge() {
    let f = stage1::fixture();
    let mut a = f.endpoint("getProjectInfoByParam").clone();
    let mut b = a.clone();
    b.summary.id = nexofolio_common::EndpointId::new();
    a.fields.retain(|field| {
        field.location == FieldLocation::RequestBody && field.path.to_string() == "id"
    });
    b.fields = a.fields.clone();
    let env = Some(f.environment());
    let link = |from, to| Link {
        id: nexofolio_common::LinkId::new(),
        from,
        to,
        from_field: Some("data.id".into()),
        to_field: Some("id".into()),
        relation: Relation::ParameterSource,
        evidence: "synthetic cycle fixture".into(),
        context: LinkContext {
            from_location: Some(FieldLocation::ResponseBody { status: 200 }),
            to_location: Some(FieldLocation::RequestBody),
            environment: env,
            confirmation: Confirmation::Observed,
            ..Default::default()
        },
        author: Author::Curate {
            pass: "fixture".into(),
            model: None,
        },
        at: "2026-10-07T00:00:00Z".parse().unwrap(),
    };
    let knowledge = vec![
        (
            a.summary.id,
            EndpointKnowledge {
                links: vec![link(b.summary.id, a.summary.id)],
                ..Default::default()
            },
        ),
        (
            b.summary.id,
            EndpointKnowledge {
                links: vec![link(a.summary.id, b.summary.id)],
                ..Default::default()
            },
        ),
    ];
    let trace = trace_upstream(
        a.summary.id,
        &[a, b],
        &knowledge,
        nexofolio_curate::TraceOptions {
            environment: env,
            known_inputs: &[],
            authenticated: true,
            max_depth: 8,
            max_steps: 20,
        },
    );
    assert!(trace.steps.iter().any(|s| s.state == "cycle"));
    assert!(trace.steps.len() < 20);

    let create = f.endpoint("saveAndSubmitConsultTechnology");
    let summaries: Vec<_> = f.facts.iter().map(|f| f.summary.clone()).collect();
    let input = endpoint_input(create, &f.signals, &summaries, vec![]);
    let mut old = EndpointNote {
        endpoint: create.summary.id,
        name: Some("有效旧名称".into()),
        purpose: Some("有效旧用途".into()),
        context: description_context(&input),
        author: Author::Curate {
            pass: "describe".into(),
            model: None,
        },
        at: "2026-10-07T00:00:00Z".parse().unwrap(),
    };
    assert!(!description_pending(&input, Some(&old)));
    let mut changed = input.clone();
    let changed_path = changed.fields[0].path.clone();
    changed.fields[0].evidence.push(FieldEvidence {
        kind: EvidenceKind::DirectSource,
        text: "新增独立证据".into(),
        endpoint: create.summary.id,
        field: changed_path,
        example: None,
        environment: env,
        options: vec![],
        source: None,
    });
    assert!(description_pending(&changed, Some(&old)));
    old.author = Author::Person {
        user: nexofolio_common::UserId::new(),
    };
    assert!(!description_pending(&changed, Some(&old)));

    let proposals = navigation_links(&f.facts, &f.signals);
    let generated = &proposals[0];
    let mut saved = Link {
        id: nexofolio_common::LinkId::new(),
        from: generated.from,
        to: generated.to,
        from_field: Some(generated.from_field.clone()),
        to_field: Some(generated.to_field.clone()),
        relation: generated.relation,
        evidence: generated.evidence.clone(),
        context: generated.context.clone(),
        author: old.author.clone(),
        at: old.at,
    };
    let mut changed = generated.clone();
    changed.evidence.push_str(" 新证据");
    assert!(relation_commands(&[changed.clone()], &[saved.clone()], Some(saved.to)).is_empty());
    saved.author = Author::Curate {
        pass: "navigation".into(),
        model: None,
    };
    assert!(
        relation_commands(&[changed], &[saved.clone()], Some(saved.to))
            .iter()
            .any(|c| matches!(c, Command::PutLink { .. }))
    );
}
