use nexofolio_access_adapter::{Postgres, PostgresAccess};
use nexofolio_access_contracts::{ExternalIdentity, LoginStore, SessionPrincipal, Sessions};
use nexofolio_common::{ProjectId, Secret, UserId};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::{str::FromStr, time::Duration};
use uuid::Uuid;

pub struct Harness {
    pub database: Postgres,
    pub store: PostgresAccess,
    pub pool: PgPool,
    pub instance: String,
}

pub fn uuid(id: impl ToString) -> Uuid {
    id.to_string().parse().unwrap()
}

impl Harness {
    pub async fn new() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect("set isolated TEST_DATABASE_URL");
        let options = PgConnectOptions::from_str(&url).unwrap();
        assert!(
            options
                .get_database()
                .unwrap_or_default()
                .starts_with("nexofolio_access_test_"),
            "this suite requires a dedicated nexofolio_access_test_* database"
        );
        let database = Postgres::new(&Secret::new(&url), 4, Duration::from_secs(2)).unwrap();
        database.migrate().await.unwrap();
        let pool = PgPool::connect_with(options.options([("search_path", "access")]))
            .await
            .unwrap();
        let store = PostgresAccess::new(database.clone(), &Secret::new("ab".repeat(32))).unwrap();
        Self {
            database,
            store,
            pool,
            instance: format!("fixture-{}", Uuid::new_v4()),
        }
    }

    pub async fn user(&self, account: &str, synced: bool) -> (SessionPrincipal, Secret) {
        let session = self
            .store
            .normal_login(&ExternalIdentity {
                instance: self.instance.clone(),
                external_id: Uuid::new_v4().to_string(),
                account: account.into(),
                display_name: "fixture user".into(),
            })
            .await
            .unwrap();
        sqlx::query("UPDATE users SET grants_synced=$2 WHERE id=$1")
            .bind(uuid(session.user.id))
            .bind(synced)
            .execute(&self.pool)
            .await
            .unwrap();
        let principal = self.store.verify_session(&session.token).await.unwrap();
        (principal, session.token)
    }

    pub async fn project(&self, instance: &str, status: &str) -> ProjectId {
        let id = ProjectId::new();
        sqlx::query("INSERT INTO projects(id,instance,external_id,name,status,last_sync_generation) VALUES($1,$2,$3,'fixture project',$4,1)")
            .bind(uuid(id)).bind(instance).bind(id.to_string()).bind(status)
            .execute(&self.pool).await.unwrap();
        id
    }

    pub async fn grant(&self, user: UserId, project: ProjectId) {
        sqlx::query("INSERT INTO user_project_access(user_id,project_id) VALUES($1,$2)")
            .bind(uuid(user))
            .bind(uuid(project))
            .execute(&self.pool)
            .await
            .unwrap();
    }

    pub async fn members(&self, user: UserId) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM user_project_access WHERE user_id=$1")
            .bind(uuid(user))
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    pub async fn rename(&self, user: UserId, account: &str) {
        sqlx::query("UPDATE users SET account=$2 WHERE id=$1")
            .bind(uuid(user))
            .bind(account)
            .execute(&self.pool)
            .await
            .unwrap();
    }

    pub async fn disable(&self, user: UserId) {
        sqlx::query("UPDATE users SET enabled=false WHERE id=$1")
            .bind(uuid(user))
            .execute(&self.pool)
            .await
            .unwrap();
    }
}
