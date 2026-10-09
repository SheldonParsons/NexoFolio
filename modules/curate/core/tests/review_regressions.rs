mod review_regressions {
    pub mod support;
}
use nexofolio_common::EnvironmentId;
use nexofolio_contracts::{endpoint::*, knowledge::*};
use nexofolio_curate::*;
use review_regressions::support::*;

#[test]
fn relations_attach_only_to_their_target_endpoint() {
    let env = EnvironmentId::new();
    let captcha = facts("/slider/gen");
    let mut check = facts("/slider/check");
    check.fields = vec![field("id")];
    let mut unrelated = facts("/unrelated/process");
    unrelated.fields = vec![field("id")];
    let signals = vec![
        sig(
            captcha.summary.id,
            env,
            "uniqueId",
            "data.uniqueId",
            "captcha-A",
            FieldLocation::ResponseBody { status: 200 },
            1,
            "",
        ),
        sig(
            check.summary.id,
            env,
            "id",
            "id",
            "captcha-A",
            FieldLocation::RequestBody,
            2,
            "",
        ),
        sig(
            unrelated.summary.id,
            env,
            "id",
            "id",
            "unrelated-B",
            FieldLocation::RequestBody,
            3,
            "",
        ),
    ];
    let summaries = vec![
        captcha.summary,
        check.summary.clone(),
        unrelated.summary.clone(),
    ];
    let positive = field_inputs(&check, &signals, &summaries);
    let negative = field_inputs(&unrelated, &signals, &summaries);
    assert!(
        positive[0]
            .evidence
            .iter()
            .any(|e| e.endpoint == signals[0].endpoint)
    );
    assert!(
        !negative[0]
            .evidence
            .iter()
            .any(|e| e.endpoint == signals[0].endpoint)
    );
}

#[test]
fn common_gateway_is_not_an_entity_but_a_resource_namespace_is() {
    let env = EnvironmentId::new();
    let source = facts("/gateway/commodity-api/v1/commodity/list");
    for (path, expected) in [
        ("/gateway/consult-api/v1/consult/detail/{id}", false),
        ("/gateway/commodity-api/v1/commodity/detail/{id}", true),
        ("/gateway/commodity-api/v1/commodity/{id}", true),
        ("/gateway/list/{id}", false),
    ] {
        let target = facts(path);
        let signals = vec![
            sig(
                source.summary.id,
                env,
                "id",
                "data[].id",
                "123",
                FieldLocation::ResponseBody { status: 200 },
                1,
                "/0",
            ),
            sig(
                target.summary.id,
                env,
                "id",
                "id",
                "123",
                FieldLocation::Path,
                2,
                "",
            ),
        ];
        let links = navigation_links(&[source.clone(), target], &signals);
        assert_eq!(!links.is_empty(), expected, "{path}");
    }
}

#[test]
fn equal_pagination_is_not_an_id_requirement_but_a_detail_id_is() {
    let env = EnvironmentId::new();
    let list = facts("/gateway/config/list");
    let add = facts("/gateway/config/add");
    let mut signals = vec![
        sig(
            list.summary.id,
            env,
            "size",
            "size",
            "20",
            FieldLocation::Query,
            1,
            "",
        ),
        sig(
            list.summary.id,
            env,
            "id",
            "data[].id",
            "20",
            FieldLocation::ResponseBody { status: 200 },
            1,
            "/0",
        ),
        sig(
            list.summary.id,
            env,
            "name",
            "data[].name",
            "配置A",
            FieldLocation::ResponseBody { status: 200 },
            1,
            "/0",
        ),
        sig(
            add.summary.id,
            env,
            "configId",
            "configs[].configId",
            "20",
            FieldLocation::RequestBody,
            2,
            "/0",
        ),
        sig(
            add.summary.id,
            env,
            "configName",
            "configs[].configName",
            "配置A",
            FieldLocation::RequestBody,
            2,
            "/0",
        ),
    ];
    let fs = vec![list, add];
    let links = navigation_links(&fs, &signals);
    let id = links.iter().find(|l| l.from_field == "data[].id").unwrap();
    assert_eq!(id.relation, Relation::ParameterSource);
    assert!(id.context.source_input.is_none());
    signals[0].name = "id".into();
    signals[0].path = "id".into();
    let links = navigation_links(&fs, &signals);
    let id = links.iter().find(|l| l.from_field == "data[].id").unwrap();
    assert_eq!(id.relation, Relation::EntityReference);
    assert_eq!(id.context.source_input.as_ref().unwrap().path, "id");
    signals[0].environment = EnvironmentId::new();
    let links = navigation_links(&fs, &signals);
    let id = links.iter().find(|l| l.from_field == "data[].id").unwrap();
    assert_eq!(
        id.relation,
        Relation::ParameterSource,
        "same example text in another environment is not this request"
    );
}

#[test]
fn positions_and_response_statuses_are_distinct_through_refresh_and_manual_protection() {
    let env = EnvironmentId::new();
    let source = facts("/gateway/commodity/list");
    let target = facts("/gateway/commodity/detail/{id}");
    let mut signals = vec![];
    for status in [200, 400] {
        signals.push(sig(
            source.summary.id,
            env,
            "id",
            "data.id",
            "123",
            FieldLocation::ResponseBody { status },
            1,
            "",
        ));
    }
    for location in [FieldLocation::Path, FieldLocation::Query] {
        signals.push(sig(
            target.summary.id,
            env,
            "id",
            "id",
            "123",
            location,
            2,
            "",
        ));
    }
    let proposals = navigation_links(&[source, target], &signals);
    assert_eq!(proposals.len(), 4);
    let mut saved: Vec<_> = proposals
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
    assert!(relation_commands(&proposals, &saved, None).is_empty());
    saved[0].author = Author::Person {
        user: nexofolio_common::UserId::new(),
    };
    let keep = &proposals[1..];
    assert!(
        relation_commands(keep, &saved, None).is_empty(),
        "manual missing edge is protected"
    );
    saved[0].author = Author::Curate {
        pass: "navigation".into(),
        model: None,
    };
    saved[0].context.recipe = Some("navigation-v2".into());
    assert!(
        matches!(relation_commands(keep,&saved,None).as_slice(),[Command::RemoveLink{id}] if *id == saved[0].id)
    );
}

#[test]
fn omitted_environment_preserves_auth_and_multiple_environments_require_a_choice() {
    let env = EnvironmentId::new();
    let target = facts("/create");
    let auth = |environment| AuthenticationUse {
        carrier: "Authorization".into(),
        scheme: "Bearer".into(),
        credential_ref: "redacted:dummy".into(),
        source: EvidenceSource {
            environment: Some(environment),
            ..Default::default()
        },
        confirmation: Confirmation::Observed,
    };
    let mut k = EndpointKnowledge {
        note: Some(EndpointNote {
            endpoint: target.summary.id,
            name: None,
            purpose: None,
            context: EndpointContext {
                authentication: vec![auth(env)],
                ..Default::default()
            },
            author: Author::Curate {
                pass: "test".into(),
                model: None,
            },
            at: "2026-10-07T00:00:00Z".parse().unwrap(),
        }),
        ..Default::default()
    };
    let run = |k: &EndpointKnowledge, environment| {
        trace_upstream(
            target.summary.id,
            std::slice::from_ref(&target),
            &[(target.summary.id, k.clone())],
            TraceOptions {
                environment,
                known_inputs: &[],
                authenticated: false,
                max_depth: 4,
                max_steps: 20,
            },
        )
    };
    assert!(
        run(&k, None)
            .steps
            .iter()
            .any(|s| s.state == "authentication_gap")
    );
    assert_eq!(run(&k, None).environment, Some(env));
    k.note
        .as_mut()
        .unwrap()
        .context
        .authentication
        .push(auth(EnvironmentId::new()));
    let ambiguous = run(&k, None);
    assert_eq!(ambiguous.steps[0].state, "environment_required");
    assert_eq!(ambiguous.available_environments.len(), 2);
    assert!(
        run(&k, Some(env))
            .steps
            .iter()
            .any(|s| s.state == "authentication_gap")
    );
    assert_eq!(
        run(&k, Some(EnvironmentId::new())).steps[0].state,
        "environment_unavailable"
    );
}
