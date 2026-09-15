//! Adapter for the verified ZenTao REST v1 shapes. No global cookie jar or password storage.
use async_trait::async_trait;
use nexofolio_access::{
    ExternalIdentity, ExternalProject, LoginCredentials, LoginProvider, ProjectSnapshot,
    RemoteLogin,
};
use nexofolio_contracts::{Error, Result, Secret};
use reqwest::{Client, Url};
use serde::Deserialize;
use std::{collections::HashSet, time::Duration};

#[derive(Clone)]
pub struct Zentao {
    base: Url,
    client: Client,
}
fn remote_error() -> Error {
    Error::Unavailable {
        component: "zentao",
    }
}
#[derive(Deserialize)]
struct TokenResponse {
    token: String,
}
#[derive(Deserialize)]
struct UserResponse {
    profile: Profile,
}
#[derive(Deserialize)]
struct Profile {
    id: u64,
    account: String,
    realname: String,
    deleted: serde_json::Value,
    #[serde(default)]
    view: Option<View>,
}
#[derive(Deserialize)]
struct View {
    projects: String,
}
#[derive(Deserialize)]
struct Projects {
    page: u32,
    total: u64,
    limit: u32,
    projects: Vec<Project>,
}
#[derive(Deserialize)]
struct Project {
    id: u64,
    name: String,
    status: String,
}

impl Zentao {
    pub fn new(base: &str) -> Result<Self> {
        let mut url =
            Url::parse(base).map_err(|_| Error::invalid("NEXOFOLIO_ZENTAO_BASE_URL is invalid"))?;
        let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
        if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(Error::invalid(
                "ZenTao requires HTTPS (HTTP is allowed only on loopback), without embedded credentials/query",
            ));
        }
        url.set_path(&format!("{}/", url.path().trim_end_matches('/')));
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(12))
            .build()
            .map_err(|_| remote_error())?;
        Ok(Self { base: url, client })
    }
    pub fn instance(&self) -> String {
        self.base.as_str().trim_end_matches('/').to_owned()
    }
    async fn json<T: serde::de::DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
        auth: bool,
    ) -> Result<T> {
        let mut response = request.send().await.map_err(|_| remote_error())?;
        if matches!(response.status().as_u16(), 401 | 403) {
            return Err(if auth {
                Error::Unauthenticated
            } else {
                remote_error()
            });
        }
        if !response.status().is_success() {
            return Err(remote_error());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| remote_error())? {
            if body.len() + chunk.len() > 8 * 1024 * 1024 {
                return Err(remote_error());
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|_| remote_error())
    }
    async fn login_inner(&self, credentials: &LoginCredentials) -> Result<RemoteLogin> {
        let response:TokenResponse=self.json(self.client.post(self.base.join("tokens").map_err(|_|remote_error())?)
            .json(&serde_json::json!({"account":credentials.account,"password":credentials.password.expose()})),true).await?;
        if response.token.trim().is_empty() {
            return Err(remote_error());
        }
        let token = Secret::new(response.token);
        let user: UserResponse = self
            .json(
                self.client
                    .get(self.base.join("user").map_err(|_| remote_error())?)
                    .header("Token", token.expose()),
                true,
            )
            .await?;
        let p = user.profile;
        if p.id == 0
            || p.account.trim().is_empty()
            || !matches!(&p.deleted, serde_json::Value::Bool(false))
                && p.deleted != serde_json::json!("0")
                && p.deleted != serde_json::json!(0)
        {
            return Err(Error::Unauthenticated);
        }
        let visible_project_ids = match p.view {
            None => None,
            Some(v) => {
                let mut ids = Vec::new();
                for id in v
                    .projects
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    let numeric = id.parse::<u64>().map_err(|_| remote_error())?;
                    if numeric == 0 {
                        return Err(remote_error());
                    }
                    ids.push(numeric.to_string());
                }
                Some(ids)
            }
        };
        Ok(RemoteLogin {
            identity: ExternalIdentity {
                instance: self.instance(),
                external_id: p.id.to_string(),
                account: p.account,
                display_name: p.realname,
            },
            credential: token,
            visible_project_ids,
        })
    }
    async fn projects_inner(&self, login: &RemoteLogin) -> Result<ProjectSnapshot> {
        let mut rows = Vec::new();
        let mut ids = HashSet::new();
        let mut expected = None;
        for page in 1..=200u32 {
            let result: Projects = self
                .json(
                    self.client
                        .get(self.base.join("projects").map_err(|_| remote_error())?)
                        .header("Token", login.credential.expose())
                        .query(&[
                            ("page", page.to_string()),
                            ("limit", "50".into()),
                            ("status", "all".into()),
                        ]),
                    false,
                )
                .await?;
            if result.page != page
                || result.limit == 0
                || result.total > 10000
                || expected.is_some_and(|n| n != result.total)
            {
                return Err(remote_error());
            }
            expected = Some(result.total);
            if result.projects.is_empty() && rows.len() as u64 != result.total {
                return Err(remote_error());
            }
            for p in result.projects {
                if p.id == 0 || !ids.insert(p.id.to_string()) {
                    return Err(remote_error());
                }
                rows.push(ExternalProject {
                    instance: self.instance(),
                    external_id: p.id.to_string(),
                    name: p.name,
                    state: p.status,
                });
            }
            if rows.len() as u64 > result.total {
                return Err(remote_error());
            }
            if rows.len() as u64 == result.total {
                let visible_project_ids = match &login.visible_project_ids {
                    Some(view) => view
                        .iter()
                        .filter(|id| ids.contains(*id))
                        .cloned()
                        .collect(),
                    None => ids.into_iter().collect(),
                };
                return Ok(ProjectSnapshot {
                    projects: rows,
                    visible_project_ids,
                });
            }
        }
        Err(remote_error())
    }
}
#[async_trait]
impl LoginProvider for Zentao {
    async fn login(&self, c: &LoginCredentials) -> Result<RemoteLogin> {
        tokio::time::timeout(Duration::from_secs(15), self.login_inner(c))
            .await
            .map_err(|_| remote_error())?
    }
    async fn projects(&self, l: &RemoteLogin) -> Result<ProjectSnapshot> {
        if l.identity.instance != self.instance() {
            return Err(Error::Forbidden);
        }
        tokio::time::timeout(Duration::from_secs(20), self.projects_inner(l))
            .await
            .map_err(|_| remote_error())?
    }
}
