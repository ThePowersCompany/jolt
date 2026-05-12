use joltr_core::{
    endpoint, Endpoint, EndpointFuture, JsonBody, Method, Request, Response, StatusCode,
};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

impl JsonBody for HealthResponse {}

#[derive(Default)]
struct HealthEndpoint;

#[endpoint("/api/health")]
impl HealthEndpoint {
    #[get]
    fn health(&self) -> Response<HealthResponse> {
        Response::new(StatusCode::Ok, HealthResponse { status: "ok" })
    }
}

#[derive(Default)]
pub(crate) struct EchoEndpoint;

impl Endpoint for EchoEndpoint {
    fn path(&self) -> &str {
        "/api/echo"
    }

    fn method(&self) -> Method {
        Method::Post
    }

    fn handler(&self, req: Request) -> EndpointFuture {
        Box::pin(async move {
            let (status, body) = echo_body(&req);
            Response::new(status, body).into()
        })
    }
}

#[derive(Default)]
pub(crate) struct ItemEndpoint;

impl Endpoint for ItemEndpoint {
    fn path(&self) -> &str {
        "/api/items/:id"
    }

    fn method(&self) -> Method {
        Method::Get
    }

    fn handler(&self, req: Request) -> EndpointFuture {
        Box::pin(async move { Response::new(StatusCode::Ok, item_body(&req)).into() })
    }
}

#[derive(Debug, PartialEq, Serialize)]
struct ItemResponse {
    id: String,
    filter: Option<String>,
}

impl JsonBody for ItemResponse {}

fn echo_body(req: &Request) -> (StatusCode, Value) {
    match req.json::<Value>() {
        Ok(body) => (StatusCode::Ok, body),
        Err(err) => (
            StatusCode::BadRequest,
            json!({
                "error": "invalid JSON body",
                "details": err.to_string(),
            }),
        ),
    }
}

fn item_body(req: &Request) -> ItemResponse {
    ItemResponse {
        id: item_id_from_path(&req.path).to_string(),
        filter: req.query("filter").map(str::to_string),
    }
}

fn item_id_from_path(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request as AxumRequest;
    use joltr_core::tower::ServiceExt;
    use std::collections::HashMap;

    fn request(
        method: Method,
        path: &str,
        query_params: HashMap<String, String>,
        body: Vec<u8>,
    ) -> Request {
        Request {
            method,
            path: path.to_string(),
            headers: Default::default(),
            query_params,
            body,
            cookies: Vec::new(),
            finished: false,
        }
    }

    #[test]
    fn health_endpoint_returns_ok_body() {
        let response = HealthEndpoint.health();

        assert_eq!(response.status, StatusCode::Ok);
        assert_eq!(response.body.status, "ok");
    }

    #[test]
    fn echo_body_returns_json_request_body() {
        let req = request(
            Method::Post,
            "/api/echo",
            HashMap::new(),
            br#"{"message":"hello"}"#.to_vec(),
        );

        let (status, body) = echo_body(&req);

        assert_eq!(status, StatusCode::Ok);
        assert_eq!(body["message"], "hello");
    }

    #[test]
    fn item_body_uses_path_segment_and_filter_query_param() {
        let req = request(
            Method::Get,
            "/api/items/42",
            HashMap::from([("filter".to_string(), "active".to_string())]),
            Vec::new(),
        );

        assert_eq!(
            item_body(&req),
            ItemResponse {
                id: "42".to_string(),
                filter: Some("active".to_string()),
            }
        );
    }

    #[tokio::test]
    async fn item_endpoint_route_returns_path_segment_and_filter_query_param() {
        let router = joltr_core::JoltRServer::new()
            .endpoint(ItemEndpoint)
            .into_router();
        let req = AxumRequest::builder()
            .method("GET")
            .uri("/api/items/42?filter=active")
            .body(Body::empty())
            .expect("request builds");

        let response = router.oneshot(req).await.expect("request succeeds");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024)
            .await
            .expect("body collects");
        let parsed: Value = serde_json::from_slice(&body).expect("valid JSON body");
        assert_eq!(parsed["id"], "42");
        assert_eq!(parsed["filter"], "active");
    }
}
