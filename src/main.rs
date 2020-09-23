mod context;
mod dataloaders;
mod schema;
mod prof;
mod auth;
mod image;
mod chat;
mod explore;
mod analytics;
mod followers;
mod for_you;
//mod time;

use crate::context::{make_shared_context, SharedContext, RedisClient};
use crate::schema::*;
use crate::auth::{Auth, auth_token};
use crate::dataloaders::{Loaders, make_loaders};
use crate::prof::*;
use async_graphql::http::{playground_source, GQLRequest, GraphQLPlaygroundConfig};
use async_graphql::{IntoQueryBuilder, QueryBuilder, QueryResponse, EmptySubscription, Schema};
use hyper::http::status::*;
use hyper::header;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server};
use std::convert::Infallible;
use std::str;
use std::sync::Arc;
use std::time::{SystemTime, Duration};
use log::{info};
use hyper::http::HeaderValue;


enum HTTPResponse {
    Ok(Response<Body>),
    Internal(String),
    Error(StatusCode, String),
}

fn ok_response(str: String) -> HTTPResponse {
    HTTPResponse::Ok(Response::new(Body::from(str)))
}

async fn index_playground(_req: Request<Body>) -> HTTPResponse {
    let src = playground_source(
        GraphQLPlaygroundConfig::new("/graphql").subscription_endpoint("/graphql"),
    );

    return ok_response(src);
}

#[derive(serde::Serialize)]
struct QueryResponseJSON {
    data: Option<serde_json::Value>,
    errors: Option<serde_json::Value>,
}

/*
fn fmt_graphql_err() {
    #[error("Parse error: {0}")]
        Parse(#[from] crate::parser::Error),

    #[error("Query error: {err}")]
    Query {
        pos: Pos,
        path: Option<serde_json::Value>,
        err: QueryError,
    },

    #[error("Rule error")]
    Rule { errors: Vec<RuleError> },
}
*/

async fn index_image(shared: &SharedContext, req: Request<Body>) -> HTTPResponse {
    let path = req.uri().path();
    let id : i32 = match path["images/".len()+1..].parse() {
        Ok(id) => id,
        Err(e) => return HTTPResponse::Error(StatusCode::BAD_REQUEST, "Could not parse image id!".to_string())
    };

    let result = match sqlx::query!("SELECT highres FROM IMAGES WHERE id=$1", id)
        .fetch_one(&shared.db)
        .await {
        Ok(result) => result.highres,
        Err(e) => return HTTPResponse::Error(StatusCode::NOT_FOUND, "".to_string())
    };

    if let Some(result) = result {
        info!("IMAGE SIZE {} kb", result.len() / 1024);

        let mut resp = Response::new(Body::from(result));
        resp.headers_mut().insert("Content-Type", HeaderValue::from_static("image/png"));

        return HTTPResponse::Ok(resp);
    } else {
        return HTTPResponse::Error(StatusCode::NOT_FOUND, "".to_string());
    }
}

async fn index_graphql(shared: Arc<SharedContext>, loaders: Arc<Loaders>, auth: Option<Auth>, req: Request<Body>) -> HTTPResponse {
    let mut prof = Prof::new();

    let full_body_bytes = match hyper::body::to_bytes(req.into_body()).await {
        Ok(r) => r,
        Err(e) => {
            return HTTPResponse::Error(
                StatusCode::BAD_REQUEST,
                format!("Could not merge body {}", e),
            )
        }
    };

    prof.log("Read body request");

    let gql_query = match str::from_utf8(&full_body_bytes) {
        Ok(r) => r,
        Err(_) => {
            return HTTPResponse::Error(
                StatusCode::BAD_REQUEST,
                "Body is not utf8 compliant".to_string(),
            )
        }
    };

    info!("{} Query body", gql_query);

    prof.log("Converted to utf8");

    let gql_request: GQLRequest = match serde_json::from_str(&gql_query) {
        Ok(r) => r,
        Err(e) => {
            return HTTPResponse::Error(
                StatusCode::BAD_REQUEST,
                format!("Could not parse json: {}", e),
            )
        }
    };

    prof.log("Parsed gql request to json");

    let mut query: QueryBuilder = match gql_request.into_query_builder().await {
        Ok(q) => q,
        Err(e) => {
            return HTTPResponse::Error(
                StatusCode::BAD_REQUEST,
                format!("Could not parse query: {}", e),
            )
        }
    };

    prof.log("Parsed gql");

    query = query.data(shared.clone()).data(loaders.clone());
    if let Some(auth) = auth {
        query = query.data(auth);
    }

    let resp: QueryResponse = match query.execute(&shared.schema).await {
        Ok(r) => r,
        Err(e) => {
            let gql_err = async_graphql::http::GQLError(&e);
            return match serde_json::to_string(&gql_err) {
                Ok(err_json) => HTTPResponse::Error(StatusCode::BAD_REQUEST, err_json),
                Err(_) => HTTPResponse::Internal("Error could not be serialized to json".to_string())
            }
        }
    };

    prof.log("Executed graphql query");

    let resp_json_obj = QueryResponseJSON {
        data: Some(resp.data),
        errors: None,
    };

    let resp_json = match serde_json::to_string(&resp_json_obj) {
        Ok(r) => r,
        Err(e) => return HTTPResponse::Internal(format!("Could not serialize query {}", e)),
    };

    prof.log("Serialized response to json");

    return ok_response(resp_json);
}

async fn route_and_auth(ctx: Arc<SharedContext>, loaders: Arc<Loaders>, req: Request<Body>) -> HTTPResponse {
    let bearer = req.headers().get("bearer");
    let auth = match bearer {
        Some(bearer) => {
            let as_str = match bearer.to_str() {
                Ok(str) => str,
                Err(e) => return HTTPResponse::Error(StatusCode::BAD_REQUEST, "Bearer token must be string".to_string())
            };

            let mut redis = ctx.redis.conn().await;
            match auth_token(&mut redis, as_str).await {
                Ok(auth) => Some(auth),
                Err(e) => return HTTPResponse::Error(StatusCode::BAD_REQUEST, "Failed to authenticate".to_string()) //todo not always the case
            }
        },
        None => None
    };

    let path = req.uri().path();
    match path {
        "/graphql" => index_graphql(ctx, loaders,auth, req).await,
        "/graphqi" => index_playground(req).await,
        _ if path.starts_with("/images") => index_image(&ctx, req).await,
        _ => HTTPResponse::Error(StatusCode::NOT_FOUND, format!("Could not find {}", path)),
    }
}

async fn index(ctx: Arc<SharedContext>, loaders: Arc<Loaders>, req: Request<Body>) -> Result<Response<Body>, Infallible> {
    let time = SystemTime::now();
    let path = req.uri().path();
    let method = req.method().to_string();
    let owned = path.to_string();

    match route_and_auth(ctx, loaders, req).await {
        HTTPResponse::Ok(mut resp) => {
            let elapsed = time.elapsed().unwrap_or_else(|_| Duration::from_millis(0));
            info!("{} [{}] - {}ms", method, StatusCode::OK, elapsed.as_millis());

            *resp.status_mut() = StatusCode::OK;
            Ok(resp)
        }
        HTTPResponse::Internal(err) => {
            info!(
                "{} [{}] {} {}",
                method,
                StatusCode::INTERNAL_SERVER_ERROR,
                owned,
                err,
            );

            let mut resp = Response::new(Body::from("Internal server error occured!"));
            *resp.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;

            Ok(resp)
        }
        HTTPResponse::Error(status, err) => {
            info!("{} [{}] {} {}", method, status, owned, err);

            let mut resp = Response::new(Body::from(err));
            *resp.status_mut() = status;

            Ok(resp)
        }
    }
}

/*
impl Service<Request<Body>> for APIRouter {
    type Request = Request<Body>;
    type Response = Response<Body>;
    type Error = hyper::Error;
    type Future = Box<Future<Output = Response<Self::Response, Self::Error>>>;

    fn poll_ready(&mut self, ctx: &Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&self, req: Request<Body>) -> Self::Future {
        match req.uri().path() {
            "/graphql" => index_graphql(&self.schema, req),
            "/graphqi" => index_playground(req),
        }
    }
}*/

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    pretty_env_logger::init();

    let shared = make_shared_context().await?;
    let loaders = make_loaders(shared.clone());

    let addr = ([0, 0, 0, 0], 8080).into();

    let service_fn = make_service_fn(move |_conn| {
        let shared = shared.clone();
        let loaders = loaders.clone();

        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                let shared = shared.clone();
                let loaders = loaders.clone();
                index(shared, loaders, req)
            }))
        }
    });


    //{"action":{"Viewed":{"Project":10}},"timestamp":"2020-09-05T08:20:28.534821Z","duration":100}
    analytics::event_json_sample();

    let server = Server::bind(&addr).serve(service_fn);

    info!("Listening on http://{}", addr);

    server.await?;

    Ok(())
}
