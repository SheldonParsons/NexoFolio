use super::*;
use nexofolio_common::{ProjectId, UserId};
#[test]
fn old_review_identity_is_recovered_without_guessing_the_creator() {
    let reviewer = UserId::new();
    let at = chrono::Utc::now();
    let old = CandidateOperationHistory {
        id: uuid::Uuid::new_v4(),
        project_id: ProjectId::new(),
        state: CandidateState::Discarded,
        created_at: at,
        updated_at: at,
        reviewed_by: Some(reviewer),
        operations: vec![],
    };
    let events = events(&old);
    assert!(matches!(events[0].actor, CandidateActor::Unknown));
    assert_eq!(events[1].actor.user_id(), Some(reviewer));
    assert_eq!(events[1].source, OperationSource::Legacy);
    let names = [ActorName {
        id: reviewer,
        display_name: "当前资料名称".into(),
    }];
    let resolved = resolve(events, &names);
    assert_eq!(resolved[1].name_source, "profile");
    assert_eq!(resolved[0].name_source, "missing");
}
#[test]
fn saved_names_and_system_identity_are_not_overwritten_by_current_profiles() {
    let id = UserId::new();
    let user = CandidateOperation::new(
        CandidateAction::Organize,
        CandidateActor::User {
            id,
            display_name: Some("当时名称".into()),
        },
        chrono::Utc::now(),
    );
    let system = CandidateOperation::new(
        CandidateAction::Apply,
        CandidateActor::System {
            id: "system".into(),
            display_name: "系统".into(),
        },
        chrono::Utc::now(),
    );
    let resolved = resolve(
        vec![user, system],
        &[ActorName {
            id,
            display_name: "新名称".into(),
        }],
    );
    assert!(
        matches!(&resolved[0].event.actor, CandidateActor::User { display_name: Some(name), .. } if name == "当时名称")
    );
    assert_eq!(resolved[0].name_source, "snapshot");
    assert!(matches!(
        resolved[1].event.actor,
        CandidateActor::System { .. }
    ));
}
