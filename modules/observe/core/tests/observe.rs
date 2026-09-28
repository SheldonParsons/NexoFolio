//! Observe end to end on the in-memory store: identities, rehoming, labels
//! and the address verdicts, as intake and the readers would see them.

use chrono::{DateTime, Utc};
use nexofolio_common::{EndpointId, EnvironmentId, ProjectId};
use nexofolio_contracts::endpoint::{
    AutoReason, Decision, EndpointChange, EndpointError, EndpointEvent, EndpointFacts,
    EndpointReader, FieldLabel, FieldLocation, PathSegment, ServiceAddress, ServiceAddresses,
    Verdict,
};
use nexofolio_contracts::feed::{ChangeFeed, Cursor};
use nexofolio_contracts::observation::{
    Body, CanonicalObservation, Content, Fact, HttpExchange, HttpRequest, HttpResponse,
    ObservationSink, SinkError,
};
use nexofolio_contracts::testing::{
    observation_sink_conformance, sample_declaration, sample_exchange,
    service_addresses_conformance,
};
use nexofolio_observe::Observe;
use nexofolio_observe_contracts::testing::InMemoryObserveStore;
use uuid::Uuid;

struct World {
    store: InMemoryObserveStore,
    observe: Observe<InMemoryObserveStore>,
    project: ProjectId,
    prod: EnvironmentId,
}

impl World {
    fn new() -> Self {
        let store = InMemoryObserveStore::new();
        Self {
            observe: Observe::new(store.clone()),
            store,
            project: ProjectId::new(),
            prod: EnvironmentId::new(),
        }
    }

    async fn send(&self, observations: Vec<CanonicalObservation>) {
        self.observe.accept(observations).await.expect("accepted");
    }

    /// `times` calls, one second apart from `from`, each in its own batch.
    async fn calls(
        &self,
        environment: EnvironmentId,
        url: &str,
        body: Body,
        times: i64,
        from: i64,
    ) {
        for n in 0..times {
            let call = self.call(environment, "GET", url, body.clone(), from + n);
            self.send(vec![call]).await;
        }
    }

    fn call(
        &self,
        environment: EnvironmentId,
        method: &str,
        url: &str,
        body: Body,
        at: i64,
    ) -> CanonicalObservation {
        let mut observation = sample_exchange(self.project, environment);
        observation.observed_at = time(at);
        observation.fact = Fact::Exchange(HttpExchange {
            request: HttpRequest {
                method: method.into(),
                url: url.into(),
                url_truncated: false,
                headers: None,
                body: Body::None,
            },
            response: Some(HttpResponse {
                status: 200,
                headers: None,
                body,
            }),
        });
        observation
    }

    async fn endpoints(&self) -> Vec<(String, EndpointId)> {
        EndpointReader::list(&self.observe, self.project)
            .await
            .expect("list")
            .into_iter()
            .map(|summary| (summary.path_template, summary.id))
            .collect()
    }

    async fn only_endpoint(&self) -> EndpointFacts {
        let endpoints = self.endpoints().await;
        assert_eq!(endpoints.len(), 1, "{endpoints:?}");
        self.get(endpoints[0].1).await
    }

    async fn get(&self, id: EndpointId) -> EndpointFacts {
        self.observe
            .get(id)
            .await
            .expect("get")
            .expect("known endpoint")
    }

    async fn events(&self) -> Vec<EndpointEvent> {
        self.store
            .read(Cursor::START, 1000)
            .await
            .expect("feed")
            .into_iter()
            .map(|change| change.event)
            .collect()
    }
}

fn time(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_790_000_000 + seconds, 0).expect("valid timestamp")
}

fn json(text: &str) -> Body {
    Body::Full {
        media_type: Some("application/json".into()),
        content: Content::Text(text.into()),
    }
}

fn label(facts: &EndpointFacts, path: &[&str]) -> Vec<FieldLabel> {
    let path: Vec<PathSegment> = path
        .iter()
        .map(|segment| match *segment {
            "[]" => PathSegment::Items,
            key => PathSegment::Key(key.into()),
        })
        .collect();
    let field = facts
        .fields
        .iter()
        .find(|field| {
            field.location == (FieldLocation::ResponseBody { status: 200 }) && field.path.0 == path
        })
        .unwrap_or_else(|| panic!("no field {path:?}"));
    field.labels.iter().map(|label| label.label).collect()
}

#[tokio::test]
async fn passes_the_shared_conformance_suites() {
    let world = World::new();
    observation_sink_conformance(&world.observe, world.project, world.prod).await;
    service_addresses_conformance(&world.observe).await;
}

#[tokio::test]
async fn a_redelivered_batch_counts_once() {
    let world = World::new();
    let url = "https://api.example.com/orders/1";
    let batch = vec![
        world.call(world.prod, "GET", url, json(r#"{"id":1}"#), 0),
        world.call(world.prod, "GET", url, json(r#"{"id":1}"#), 1),
    ];
    world.send(batch.clone()).await;
    world.send(batch).await;
    assert_eq!(world.store.total_calls(), 2);
    let mut other = world.call(world.prod, "GET", url, json(r#"{"id":1}"#), 2);
    other.key.batch_id = Uuid::new_v4();
    world.send(vec![other]).await;
    assert_eq!(world.store.total_calls(), 3);
}

#[tokio::test]
async fn an_unavailable_store_is_retryable() {
    let world = World::new();
    world.store.set_unavailable(true);
    let call = sample_exchange(world.project, world.prod);
    assert_eq!(
        world.observe.accept(vec![call]).await,
        Err(SinkError::Unavailable)
    );
    assert_eq!(
        EndpointReader::list(&world.observe, world.project).await,
        Err(EndpointError::Unavailable)
    );
    assert_eq!(
        world
            .observe
            .decide(
                world.project,
                ServiceAddress::parse("https://api.example.com").unwrap(),
                None
            )
            .await,
        Err(EndpointError::Unavailable)
    );
}

#[tokio::test]
async fn announces_new_endpoints_and_new_structures_once() {
    let world = World::new();
    let url = "https://api.example.com/orders/7";
    world
        .calls(world.prod, url, json(r#"{"id":7}"#), 3, 0)
        .await;
    let id = world.only_endpoint().await.summary.id;
    let changes: Vec<EndpointChange> = world.events().await.into_iter().map(|e| e.change).collect();
    assert_eq!(
        changes,
        vec![
            EndpointChange::Created {
                method: "GET".into(),
                path_template: "/orders/{id}".into()
            },
            EndpointChange::StructureChanged {
                environment_id: Some(world.prod)
            },
        ]
    );
    world
        .calls(world.prod, url, json(r#"{"id":7,"paid":true}"#), 1, 10)
        .await;
    let last = world.events().await.pop().expect("event");
    assert_eq!(last.endpoint, id);
    assert_eq!(
        last.change,
        EndpointChange::StructureChanged {
            environment_id: Some(world.prod)
        }
    );
}

#[tokio::test]
async fn a_late_declaration_absorbs_observed_traffic_and_learns_the_base_path() {
    let world = World::new();
    world
        .calls(
            world.prod,
            "https://example.test/api/order/1001",
            json(r#"{"id":1}"#),
            2,
            0,
        )
        .await;
    let observed = world.only_endpoint().await.summary;
    assert_eq!(observed.path_template, "/api/order/{id}");

    world.send(vec![sample_declaration(world.project)]).await;
    let declared = world.only_endpoint().await;
    assert_eq!(declared.summary.path_template, "/order/{orderId}");
    assert!(declared.summary.declared);
    assert_eq!(declared.aliases, vec![observed.id]);
    assert_eq!(declared.addresses[0].base_path, "/api");
    assert_eq!(
        world.get(observed.id).await.summary.id,
        declared.summary.id,
        "old id still resolves"
    );
    assert!(world.events().await.contains(&EndpointEvent {
        project_id: world.project,
        endpoint: observed.id,
        change: EndpointChange::MergedInto {
            into: declared.summary.id
        },
    }));

    world
        .calls(
            world.prod,
            "https://example.test/api/order/1002",
            json(r#"{"id":2}"#),
            1,
            10,
        )
        .await;
    let after = world.only_endpoint().await;
    assert_eq!(
        after.summary.id, declared.summary.id,
        "later calls land on the declaration"
    );
    assert_eq!(after.summary.environments[0].calls, 3);
}

#[tokio::test]
async fn a_declaration_first_teaches_the_base_path_on_first_traffic() {
    let world = World::new();
    world.send(vec![sample_declaration(world.project)]).await;
    world
        .calls(
            world.prod,
            "https://example.test/gateway/order/5",
            json(r#"{"id":5}"#),
            1,
            0,
        )
        .await;
    let facts = world.only_endpoint().await;
    assert_eq!(facts.summary.path_template, "/order/{orderId}");
    assert_eq!(facts.addresses[0].base_path, "/gateway");
}

#[tokio::test]
async fn labels_fields_from_enough_calls() {
    let world = World::new();
    let url = "https://api.example.com/orders/1";
    world
        .calls(world.prod, url, json(r#"{"id":1,"note":"x"}"#), 5, 0)
        .await;
    world
        .calls(world.prod, url, json(r#"{"id":1}"#), 5, 2)
        .await;
    let facts = world.only_endpoint().await;
    assert_eq!(label(&facts, &["id"]), vec![FieldLabel::Always]);
    assert_eq!(label(&facts, &["note"]), vec![FieldLabel::Optional]);

    let fresh = World::new();
    fresh
        .calls(fresh.prod, url, json(r#"{"id":1}"#), 4, 0)
        .await;
    assert_eq!(
        label(&fresh.only_endpoint().await, &["id"]),
        vec![FieldLabel::Observing]
    );
}

#[tokio::test]
async fn truncated_bodies_and_empty_lists_do_not_prove_absence() {
    let world = World::new();
    let url = "https://api.example.com/orders";
    world
        .calls(
            world.prod,
            url,
            json(r#"{"total":1,"list":[{"sku":"A-1"}]}"#),
            5,
            0,
        )
        .await;
    world
        .calls(world.prod, url, json(r#"{"total":0,"list":[]}"#), 5, 2)
        .await;
    let truncated = Body::Truncated {
        media_type: Some("application/json".into()),
        content: Content::Text(r#"{"list":[{"sku":"#.into()),
        original_bytes: Some(4096),
    };
    world.calls(world.prod, url, truncated, 5, 4).await;
    let facts = world.only_endpoint().await;
    assert_eq!(label(&facts, &["total"]), vec![FieldLabel::Always]);
    assert_eq!(
        label(&facts, &["list", "[]", "sku"]),
        vec![FieldLabel::Always]
    );
}

#[tokio::test]
async fn a_field_missing_in_one_environment_differs() {
    let world = World::new();
    let test = EnvironmentId::new();
    let url = "https://api.example.com/orders/1";
    world
        .calls(world.prod, url, json(r#"{"id":1,"debug":"x"}"#), 5, 0)
        .await;
    world.calls(test, url, json(r#"{"id":1}"#), 5, 0).await;
    let facts = world.only_endpoint().await;
    let debug = facts
        .fields
        .iter()
        .find(|field| field.path.0 == vec![PathSegment::Key("debug".into())])
        .expect("debug field");
    assert!(debug.differs_between_environments);
    assert_eq!(facts.summary.environments.len(), 2);
}

#[tokio::test]
async fn non_json_addresses_are_external_with_absolute_templates() {
    let world = World::new();
    let html = Body::Full {
        media_type: Some("text/html".into()),
        content: Content::Text("<p>ok</p>".into()),
    };
    world
        .calls(
            world.prod,
            "https://analytics.example.net/collect/123",
            html,
            1,
            0,
        )
        .await;
    let facts = world.only_endpoint().await;
    assert_eq!(
        facts.summary.path_template,
        "https://analytics.example.net/collect/{id}"
    );
    assert!(facts.summary.external);
    let statuses = ServiceAddresses::list(&world.observe, world.project)
        .await
        .unwrap();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].verdict, Verdict::External);
    assert_eq!(statuses[0].decision, Decision::Auto(AutoReason::NotJson));
    assert_eq!(statuses[0].calls, 1);
}

#[tokio::test]
async fn a_registered_address_applies_from_the_first_call() {
    let world = World::new();
    let address = ServiceAddress::parse("https://pay.example.org").unwrap();
    world
        .observe
        .decide(world.project, address, Some(Verdict::External))
        .await
        .unwrap();
    world
        .calls(
            world.prod,
            "https://pay.example.org/v1/charge",
            json("{}"),
            1,
            0,
        )
        .await;
    assert_eq!(
        world.only_endpoint().await.summary.path_template,
        "https://pay.example.org/v1/charge"
    );
}

#[tokio::test]
async fn changing_a_verdict_moves_the_traffic() {
    let world = World::new();
    world
        .calls(
            world.prod,
            "https://api.example.com/orders/1",
            json(r#"{"id":1}"#),
            2,
            0,
        )
        .await;
    let own = world.only_endpoint().await.summary;
    assert_eq!(own.path_template, "/orders/{id}");

    let address = ServiceAddress::parse("https://api.example.com").unwrap();
    world
        .observe
        .decide(world.project, address, Some(Verdict::External))
        .await
        .unwrap();
    let external = world.only_endpoint().await;
    assert_eq!(
        external.summary.path_template,
        "https://api.example.com/orders/{id}"
    );
    assert_eq!(external.summary.environments[0].calls, 2);
    assert_eq!(external.aliases, vec![own.id]);
    let statuses = ServiceAddresses::list(&world.observe, world.project)
        .await
        .unwrap();
    assert_eq!(statuses[0].decision, Decision::Manual);
}
