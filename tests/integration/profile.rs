//! Personal presentation persistence/ownership over an isolated PostgreSQL database.
use image::{ExtendedColorType, ImageEncoder, codecs::webp::WebPEncoder};
use nexofolio_access::{ProfileService, normalize_profile};
use nexofolio_access_adapter::{Postgres, PostgresAccess};
use nexofolio_access_contracts::{
    ExternalIdentity, LoginStore, PersonalProfiles, ProfileUpdate, SessionPrincipal, Sessions,
};
use nexofolio_common::{Error, Secret};
use std::{sync::Arc, time::Duration};

#[tokio::test]
#[ignore = "requires isolated TEST_DATABASE_URL"]
async fn self_profile_is_persistent_versioned_and_independent_of_provider_identity_and_other_users()
{
    let url = Secret::new(std::env::var("TEST_DATABASE_URL").unwrap());
    let database = Postgres::new(&url, 4, Duration::from_secs(5)).unwrap();
    database.migrate().await.unwrap();
    let store =
        Arc::new(PostgresAccess::new(database.clone(), &Secret::new("ab".repeat(32))).unwrap());
    let user = ExternalIdentity {
        instance: "profile-fixture".into(),
        external_id: "alice".into(),
        account: "alice".into(),
        display_name: "账号姓名".into(),
    };
    let login = store.normal_login(&user).await.unwrap();
    let principal = SessionPrincipal {
        user_id: login.user.id,
        instance: user.instance.clone(),
    };
    let bob = store
        .normal_login(&ExternalIdentity {
            external_id: "bob".into(),
            account: "bob".into(),
            display_name: "Bob".into(),
            ..user.clone()
        })
        .await
        .unwrap();
    let bob_p = SessionPrincipal {
        user_id: bob.user.id,
        instance: user.instance.clone(),
    };
    let service = ProfileService::new(store.clone());
    let before = service.personal_profile(&principal).await.unwrap();
    assert_eq!(before.version, 0);
    let saved = service
        .update_profile(
            &principal,
            ProfileUpdate {
                display_name: Some(" 个人展示 ".into()),
                bio: " 简介 ".into(),
                version: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(saved.user.display_name, "个人展示");
    assert_eq!(saved.account_display_name, "账号姓名");
    assert_eq!(saved.user.account, "alice");
    assert_eq!(saved.bio, "简介");
    assert_eq!(saved.version, 1);
    assert_eq!(store.me(&principal).await.unwrap().display_name, "个人展示");
    assert_eq!(
        service
            .personal_profile(&bob_p)
            .await
            .unwrap()
            .user
            .display_name,
        "Bob"
    );
    assert!(matches!(
        service
            .update_profile(
                &principal,
                ProfileUpdate {
                    display_name: Some("冲突写入".into()),
                    bio: "".into(),
                    version: 0
                }
            )
            .await,
        Err(Error::Conflict)
    ));
    store
        .normal_login(&ExternalIdentity {
            display_name: "新的账号姓名".into(),
            ..user
        })
        .await
        .unwrap();
    let after_login = service.personal_profile(&principal).await.unwrap();
    assert_eq!(after_login.account_display_name, "新的账号姓名");
    assert_eq!(after_login.user.display_name, "个人展示");
    let mut bytes = Vec::new();
    WebPEncoder::new_lossless(&mut bytes)
        .write_image(&vec![88; 512 * 512 * 3], 512, 512, ExtendedColorType::Rgb8)
        .unwrap();
    let avatar = service
        .update_avatar(&principal, 1, Some(bytes))
        .await
        .unwrap();
    assert!(
        avatar
            .user
            .avatar_url
            .as_ref()
            .unwrap()
            .starts_with("data:image/webp;base64,")
    );
    assert_eq!(
        store.me(&principal).await.unwrap().avatar_url,
        avatar.user.avatar_url
    );
    assert!(
        service
            .personal_profile(&bob_p)
            .await
            .unwrap()
            .user
            .avatar_url
            .is_none()
    );

    assert!(
        service
            .update_avatar(&principal, 2, Some(b"invalid".to_vec()))
            .await
            .is_err()
    );
    assert_eq!(
        service.personal_profile(&principal).await.unwrap().version,
        2
    );
    let restored = service
        .update_profile(
            &principal,
            ProfileUpdate {
                display_name: None,
                bio: "简介".into(),
                version: 2,
            },
        )
        .await
        .unwrap();
    assert_eq!(restored.user.display_name, "新的账号姓名");
    assert!(
        service
            .personal_profile(&SessionPrincipal {
                instance: "wrong".into(),
                ..principal
            })
            .await
            .is_err()
    );
    drop(service);
    drop(store);
    database.close().await;
}
#[test]
fn profile_validation_is_unicode_aware_and_cannot_modify_identity_fields() {
    assert!(
        normalize_profile(ProfileUpdate {
            display_name: Some("名".repeat(40)),
            bio: "介绍".into(),
            version: 0
        })
        .is_ok()
    );
    assert!(
        normalize_profile(ProfileUpdate {
            display_name: Some("名".repeat(41)),
            bio: "".into(),
            version: 0
        })
        .is_err()
    );
    assert!(
        normalize_profile(ProfileUpdate {
            display_name: Some("不可\n换行".into()),
            bio: "".into(),
            version: 0
        })
        .is_err()
    );
    assert!(
        normalize_profile(ProfileUpdate {
            display_name: None,
            bio: "长".repeat(161),
            version: 0
        })
        .is_err()
    );
    assert!(
        serde_json::from_str::<ProfileUpdate>(
            r#"{"display_name":"name","bio":"","version":0,"account":"other"}"#
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<ProfileUpdate>(
            r#"{"display_name":"name","bio":"","version":0,"user_id":"other"}"#
        )
        .is_err()
    );
}
