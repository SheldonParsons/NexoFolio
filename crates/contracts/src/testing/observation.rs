use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nexofolio_common::{EnvironmentId, ProjectId};
use uuid::Uuid;

use crate::observation::{
    Body, CanonicalObservation, Content, DeclaredParameter, DeclaredRequest, Fact, HttpDeclaration,
    HttpExchange, HttpRequest, HttpResponse, ObservationKey, ObservationSink, SinkError, Source,
};

/// Fake observe: keeps accepted observations, ignores repeated keys, and can
/// be told to fail so intake's retry path can be tested.
#[derive(Default)]
pub struct RecordingSink {
    accepted: Mutex<BTreeMap<ObservationKey, CanonicalObservation>>,
    failures_left: Mutex<usize>,
}

impl RecordingSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// The next `times` calls to `accept` fail with [`SinkError::Unavailable`].
    pub fn fail_next(&self, times: usize) {
        *self.failures_left.lock().expect("fake lock") = times;
    }

    /// Everything accepted so far, ordered by key.
    pub fn accepted(&self) -> Vec<CanonicalObservation> {
        self.accepted
            .lock()
            .expect("fake lock")
            .values()
            .cloned()
            .collect()
    }
}

#[async_trait]
impl ObservationSink for RecordingSink {
    async fn accept(&self, observations: Vec<CanonicalObservation>) -> Result<(), SinkError> {
        {
            let mut left = self.failures_left.lock().expect("fake lock");
            if *left > 0 {
                *left -= 1;
                return Err(SinkError::Unavailable);
            }
        }
        let mut accepted = self.accepted.lock().expect("fake lock");
        for observation in observations {
            accepted.entry(observation.key).or_insert(observation);
        }
        Ok(())
    }
}

fn observed_at() -> DateTime<Utc> {
    DateTime::from_timestamp(1_790_000_000, 0).expect("valid timestamp")
}

fn source(platform: &str) -> Source {
    Source {
        platform: platform.into(),
        platform_version: None,
        source_url: None,
    }
}

/// A captured `GET /api/order/{id}/items` call with a full JSON response.
pub fn sample_exchange(
    project_id: ProjectId,
    environment_id: EnvironmentId,
) -> CanonicalObservation {
    CanonicalObservation {
        key: ObservationKey {
            batch_id: Uuid::new_v4(),
            record_id: Uuid::new_v4(),
        },
        project_id,
        environment_id: Some(environment_id),
        site: None,
        source: source("nexofolio-fetcher"),
        observed_at: observed_at(),
        fact: Fact::Exchange(HttpExchange {
            request: HttpRequest {
                method: "GET".into(),
                url: "https://example.test/api/order/1001/items?page=1".into(),
                url_truncated: false,
                headers: None,
                body: Body::None,
            },
            response: Some(HttpResponse {
                status: 200,
                headers: None,
                body: Body::Full {
                    media_type: Some("application/json".into()),
                    content: Content::Text(r#"{"list":[{"sku":"A-1","qty":2}]}"#.into()),
                },
            }),
        }),
        context: None,
    }
}

/// A project-wide declaration of `GET /order/{orderId}` with structure only.
pub fn sample_declaration(project_id: ProjectId) -> CanonicalObservation {
    let order_id = DeclaredParameter {
        required: true,
        schema: Some(serde_json::json!({ "type": "integer" })),
        ..DeclaredParameter::default()
    };
    CanonicalObservation {
        key: ObservationKey {
            batch_id: Uuid::new_v4(),
            record_id: Uuid::new_v4(),
        },
        project_id,
        environment_id: None,
        site: None,
        source: source("swagger-sync"),
        observed_at: observed_at(),
        fact: Fact::Declaration(HttpDeclaration {
            method: "GET".into(),
            path_template: "/order/{orderId}".into(),
            operation_id: None,
            summary: None,
            description: None,
            tags: Vec::new(),
            deprecated: false,
            request: DeclaredRequest {
                path_params: BTreeMap::from([("orderId".to_owned(), order_id)]),
                ..DeclaredRequest::default()
            },
            responses: BTreeMap::new(),
        }),
        context: None,
    }
}

/// Behaviour every [`ObservationSink`] must show. `project` and `environment`
/// must exist in the sink's world. Panics on the first violation.
pub async fn observation_sink_conformance<S: ObservationSink>(
    sink: &S,
    project: ProjectId,
    environment: EnvironmentId,
) {
    sink.accept(Vec::new())
        .await
        .expect("empty delivery is a no-op");
    let batch = vec![
        sample_exchange(project, environment),
        sample_declaration(project),
    ];
    sink.accept(batch.clone()).await.expect("first delivery");
    sink.accept(batch)
        .await
        .expect("redelivery of the same keys is accepted");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_passes_observation_sink_conformance() {
        let sink = RecordingSink::new();
        observation_sink_conformance(&sink, ProjectId::new(), EnvironmentId::new()).await;
        assert_eq!(sink.accepted().len(), 2, "redelivery did not duplicate");
    }

    #[tokio::test]
    async fn fake_can_simulate_outages() {
        let sink = RecordingSink::new();
        sink.fail_next(1);
        let batch = vec![sample_declaration(ProjectId::new())];
        assert_eq!(
            sink.accept(batch.clone()).await,
            Err(SinkError::Unavailable)
        );
        assert!(sink.accepted().is_empty());
        sink.accept(batch).await.unwrap();
        assert_eq!(sink.accepted().len(), 1);
    }
}
