//! Seeds only a disposable database for the real browser -> API -> audit store acceptance test.
#[allow(dead_code)]
#[path = "project_management/support.rs"]
mod support;
use nexofolio_access_contracts::Environments;
use serde_json::json;
use support::*;

#[tokio::test]
#[ignore = "requires disposable TEST_DATABASE_URL and private AUDIT_BROWSER_FIXTURE path"]
async fn seed_page_visit_fixture() {
    let h = Harness::new().await;
    let admin = h.user("A80646").await;
    let member = h.user("page-audit-member").await;
    let project = h.project("页面审计验收项目").await;
    h.grant(&member, project).await;
    let env = h
        .store
        .create_environment(&admin.principal, project, "合成测试环境")
        .await
        .unwrap();
    let other_env = h
        .store
        .create_environment(&admin.principal, project, "另一测试环境")
        .await
        .unwrap();
    let (_, endpoints) = h.populate(project, env.id).await;
    let folder = uuid::Uuid::new_v4();
    let author = json!({"by":"person","user":admin.principal.user_id});
    sqlx::query(
        "INSERT INTO knowledge.folders(id,project_id,name,author) VALUES($1,$2,'合成业务目录',$3)",
    )
    .bind(folder)
    .bind(uuid(project))
    .bind(&author)
    .execute(&h.sql)
    .await
    .unwrap();
    for (index, id) in endpoints.iter().enumerate() {
        sqlx::query("INSERT INTO knowledge.endpoint_notes(endpoint_id,project_id,name,purpose,author) VALUES($1,$2,$3,'page telemetry fixture',$4)").bind(id).bind(uuid(project)).bind(format!("合成接口 {}",index+1)).bind(&author).execute(&h.sql).await.unwrap();
        sqlx::query("INSERT INTO knowledge.placements(endpoint_id,project_id,folder_id,author) VALUES($1,$2,$3,$4)").bind(id).bind(uuid(project)).bind(folder).bind(&author).execute(&h.sql).await.unwrap();
    }
    let path = std::path::PathBuf::from(
        std::env::var("AUDIT_BROWSER_FIXTURE").expect("private fixture path"),
    );
    assert!(
        path.parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("nexofolio-page-audit.")
    );
    std::fs::write(&path,serde_json::to_vec(&json!({"instance":h.instance,"project":project,"environment":env.id,"other_environment":other_env.id,"folder":folder,"endpoints":endpoints,"member":{"id":member.principal.user_id,"token":member.token.expose()},"admin":{"id":admin.principal.user_id,"token":admin.token.expose()}})).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}
