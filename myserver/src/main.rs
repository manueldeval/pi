use std::{collections::HashMap, env, sync::Arc};

use axum::{ body::{self, Body}, extract::{Path, Query, State}, http::{header, HeaderMap, HeaderValue, Response, StatusCode}, response::{IntoResponse, Redirect}, routing::{get, post}, Json, Router};
use include_dir::{Dir,include_dir};
use tokio::signal;

static STATIC_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/static");

pub async fn status_handler() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "success",
        "message": "Pi ❤️ Rust!"
    }))
}

pub async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn static_path(Path(path): Path<String>) -> impl IntoResponse {
    let path = path.trim_start_matches('/');
    let mime_type = mime_guess::from_path(path).first_or_text_plain();

    match STATIC_DIR.get_file(path) {
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap(),
        Some(file) => Response::builder()
            .status(StatusCode::OK)
            .header(
                header::CONTENT_TYPE,
                HeaderValue::from_str(mime_type.as_ref()).unwrap(),
            )
            .body(Body::from(file.contents()))
            .unwrap(),
    }
}

pub struct AppState {
    token: String,
}


#[tokio::main]
pub async fn main() {
    let port = env::var("PORT")
        .map(|s| s.parse::<u16>().expect(format!("Unable to parse the env var PORT: {}",s).as_str()))
        .unwrap_or(3000);
    let token = env::var("TOKEN").expect("No TOKEN env var found.");
    println!("TOKEN: {}",token);
    let shared_state = Arc::new(AppState { token });


    println!("Server started successfully on port {}",port);
    
    let route = Router::new()
        .route("/static/*path", get(static_path))
        .route("/", get(|| async { Redirect::permanent("/static/love.png") }))
        .route("/api/status", get(status_handler))
        .route("/health", get(health_handler))
        .route("/api/kubectl",post(kubectl_handler)).with_state(shared_state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}",port)).await.unwrap();
    
    axum::serve(listener, route).with_graceful_shutdown(shutdown_signal()).await.unwrap();
}

async fn shutdown_signal() {
    let ctrl_c = async { signal::ctrl_c().await.expect("failed to install Ctrl+C handler"); };

    #[cfg(unix)]
    let terminate = async { signal::unix::signal(signal::unix::SignalKind::terminate()).expect("failed to install signal handler").recv().await; };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

/*
--------------------------------
KUBECTL
--------------------------------
*/
pub async fn kubectl_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
    body: String,
) -> impl IntoResponse {

    let Some(query) = params.get("query") else {
        return (StatusCode::BAD_REQUEST,Json(serde_json::json!({
            "message": "query parameter not found"
        })))
    };

    let Some(token) = headers.get("X_API_TOKEN") else {
        return (StatusCode::FORBIDDEN,Json(serde_json::json!({
            "message": "No api token found"
        })))
    };

    if token != &state.token {
        return (StatusCode::FORBIDDEN,Json(serde_json::json!({
            "message": "Bad api token"
        })))
    }
    
    match kubectl(query,&body).await {
        Ok(result) =>     (StatusCode::OK,Json(serde_json::json!({ 
            "response": result,
            "query": query,
            "body": body,
        }))),
        Err(e) =>    (StatusCode::INTERNAL_SERVER_ERROR,Json(serde_json::json!({ 
            "response": format!("Error: {}",e),
            "query": query,
            "body": body,
        }))),
    }

}



use anyhow::{bail, Context, Result};
use futures::{StreamExt, TryStreamExt};
use k8s_openapi::{apimachinery::pkg::apis::meta::v1::Time, chrono::Utc};
use kube::{
    api::{Api, DynamicObject, ListParams, Patch, PatchParams, ResourceExt},
    core::GroupVersionKind,
    discovery::{ApiCapabilities, ApiResource, Discovery, Scope},
    Client,
};

#[derive(clap::Parser)]
struct App {
    #[arg(long, short = 'l')]
    selector: Option<String>,
    #[arg(long, short)]
    namespace: Option<String>,
    #[arg(long, short = 'A')]
    all: bool,
    verb: Verb,
    resource: Option<String>,
    name: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug, clap::ValueEnum)]
enum Verb {
    Get,
    Delete,
    Apply,
}

fn resolve_api_resource(discovery: &Discovery, name: &str) -> Option<(ApiResource, ApiCapabilities)> {
    // iterate through groups to find matching kind/plural names at recommended versions
    // and then take the minimal match by group.name (equivalent to sorting groups by group.name).
    // this is equivalent to kubectl's api group preference
    discovery
        .groups()
        .flat_map(|group| {
            group
                .resources_by_stability()
                .into_iter()
                .map(move |res| (group, res))
        })
        .filter(|(_, (res, _))| {
            // match on both resource name and kind name
            // ideally we should allow shortname matches as well
            name.eq_ignore_ascii_case(&res.kind) || name.eq_ignore_ascii_case(&res.plural)
        })
        .min_by_key(|(group, _res)| group.name())
        .map(|(_, res)| res)
}

impl App {
    async fn get(&self, api: Api<DynamicObject>, lp: ListParams) -> Result<String> {
        let mut result: Vec<_> = if let Some(n) = &self.name {
            vec![api.get(n).await?]
        } else {
            api.list(&lp).await?.items
        };
        result.iter_mut().for_each(|x| x.managed_fields_mut().clear()); // hide managed fields
        Ok(serde_yaml::to_string(&result)?)
    }

    async fn delete(&self, api: Api<DynamicObject>, lp: ListParams) -> Result<String> {
        if let Some(n) = &self.name {
            api.delete(n, &Default::default()).await?;
        } else {
            api.delete_collection(&Default::default(), &lp).await?;
        }
        Ok("deleted".to_string())
    }

    async fn apply(&self, client: Client, discovery: &Discovery, yaml: &String) -> Result<String> {
        let ssapply = PatchParams::apply("kubectl-light").force();
         for doc in multidoc_deserialize(&yaml)? {
            let obj: DynamicObject = serde_yaml::from_value(doc)?;
            let namespace = obj.metadata.namespace.as_deref().or(self.namespace.as_deref());
            let gvk = if let Some(tm) = &obj.types {
                GroupVersionKind::try_from(tm)?
            } else {
                bail!("cannot apply object without valid TypeMeta {:?}", obj);
            };
            let name = obj.name_any();
            if let Some((ar, caps)) = discovery.resolve_gvk(&gvk) {
                let api = dynamic_api(ar, caps, client.clone(), namespace, false);
                println!("Applying {}: \n{}", gvk.kind, serde_yaml::to_string(&obj)?);
                let data: serde_json::Value = serde_json::to_value(&obj)?;
                let _r = api.patch(&name, &ssapply, &Patch::Apply(data)).await?;
                println!("applied {} {}", gvk.kind, name);
            } else {
                println!("Cannot apply document for unknown {:?}", gvk);
            }
        }
        Ok("Applied.".to_string())
    }
}


async fn kubectl(query: &String, body: &String) -> Result<String> {
    let full_command =  format!("kubectl {}",&query);
    let query_splitted = full_command.split_whitespace().collect::<Vec<&str>>();

    let app: App = clap::Parser::try_parse_from(query_splitted)?;

    let client = Client::try_default().await?;

    // discovery (to be able to infer apis from kind/plural only)
    let discovery = Discovery::new(client.clone()).run().await?;

    // Defer to methods for verbs
    let output_text = if let Some(resource) = &app.resource {
        // Common discovery, parameters, and api configuration for a single resource
        let (ar, caps) = resolve_api_resource(&discovery, resource)
            .with_context(|| format!("resource {resource:?} not found in cluster"))?;
        let mut lp = ListParams::default();
        if let Some(label) = &app.selector {
            lp = lp.labels(label);
        }

        let api = dynamic_api(ar, caps, client, app.namespace.as_deref(), app.all);

        match app.verb {
            Verb::Get => app.get(api, lp).await?,
            Verb::Delete => app.delete(api, lp).await?,
            Verb::Apply => bail!("verb {:?} cannot act on an explicit resource", app.verb),
        }
    } else if app.verb == Verb::Apply {
        app.apply(client, &discovery, body).await? // multi-resource special behaviour
    } else {
        bail!("Invalid command.")
    };
    Ok(output_text)
}

fn dynamic_api(
    ar: ApiResource,
    caps: ApiCapabilities,
    client: Client,
    ns: Option<&str>,
    all: bool,
) -> Api<DynamicObject> {
    if caps.scope == Scope::Cluster || all {
        Api::all_with(client, &ar)
    } else if let Some(namespace) = ns {
        Api::namespaced_with(client, namespace, &ar)
    } else {
        Api::default_namespaced_with(client, &ar)
    }
}

fn format_creation(time: Time) -> String {
    let dur = Utc::now().signed_duration_since(time.0);
    match (dur.num_days(), dur.num_hours(), dur.num_minutes()) {
        (days, _, _) if days > 0 => format!("{days}d"),
        (_, hours, _) if hours > 0 => format!("{hours}h"),
        (_, _, mins) => format!("{mins}m"),
    }
}

pub fn multidoc_deserialize(data: &str) -> Result<Vec<serde_yaml::Value>> {
    use serde::Deserialize;
    let mut docs = vec![];
    for de in serde_yaml::Deserializer::from_str(data) {
        docs.push(serde_yaml::Value::deserialize(de)?);
    }
    Ok(docs)
}
