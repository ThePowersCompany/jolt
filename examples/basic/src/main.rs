use axum::body::Body;
use axum::http::{Request as AxumRequest, StatusCode as AxumStatusCode};
use axum::response::Response as AxumResponse;
use joltr_core::{
    tower::{service_fn, Layer},
    CorsConfig, FileServeLayer, JoltRServer, Method,
};
use joltr_db::{DbConfig, JoltRDb};

mod endpoints;
mod tasks;

const DEFAULT_PORT: u16 = 3000;
const MIGRATIONS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");
const PUBLIC_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/public");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    migrate_database().await?;

    let mut scheduler = tasks::scheduler();
    scheduler.start();

    let cors = CorsConfig {
        allow_origins: vec!["http://localhost:5173".to_string()],
        allow_methods: vec![Method::Get, Method::Post, Method::Options],
        allow_headers: vec!["authorization".to_string(), "content-type".to_string()],
        max_age: 600,
        expose_headers: Vec::new(),
    };

    JoltRServer::new()
        .port(DEFAULT_PORT)
        .cors(cors)
        .endpoint(endpoints::TemplateEndpoint::new()?)
        .endpoint(endpoints::EchoEndpoint)
        .endpoint(endpoints::ItemEndpoint)
        .start(static_assets_router())
        .await?;

    Ok(())
}

fn static_assets_router() -> axum::Router {
    let not_found = service_fn(|_req: AxumRequest<Body>| async {
        Ok::<_, std::convert::Infallible>(
            AxumResponse::builder()
                .status(AxumStatusCode::NOT_FOUND)
                .body(Body::empty())
                .expect("static fallback response builds"),
        )
    });

    axum::Router::new()
        .fallback_service(FileServeLayer::new("/static", PUBLIC_DIR).layer(not_found))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use joltr_core::tower::ServiceExt;

    #[tokio::test]
    async fn serving_router_serves_public_asset_from_static_prefix() {
        let router = JoltRServer::new().build_serving_router(static_assets_router());

        let response = router
            .oneshot(
                AxumRequest::builder()
                    .uri("/static/app.css")
                    .body(Body::empty())
                    .expect("static request builds"),
            )
            .await
            .expect("router responds");

        assert_eq!(response.status(), AxumStatusCode::OK);
        let body = to_bytes(response.into_body(), 1024)
            .await
            .expect("body collects");
        let body = std::str::from_utf8(&body).expect("asset is utf-8");
        assert!(body.contains(".joltr-basic-example"));
    }
}

async fn migrate_database() -> Result<(), Box<dyn std::error::Error>> {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        println!("DATABASE_URL is not set; skipping example database migrations");
        return Ok(());
    };

    if database_url.trim().is_empty() {
        println!("DATABASE_URL is empty; skipping example database migrations");
        return Ok(());
    }

    let db = JoltRDb::connect(&DbConfig::new(database_url)).await?;
    let applied = db.migrate(MIGRATIONS_DIR).await?;
    println!("applied {applied} example database migration(s)");

    Ok(())
}
