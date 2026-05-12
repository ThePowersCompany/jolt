use axum::body::Body;
use axum::http::{Request as AxumRequest, StatusCode as AxumStatusCode};
use axum::response::Response as AxumResponse;
use joltr_core::{
    tower::{service_fn, Layer},
    CorsConfig, FileServeLayer, JoltRServer, Method, TlsConfig,
};
use joltr_db::{DbConfig, JoltRDb};
use std::{io, path::PathBuf};

mod endpoints;
mod tasks;

const DEFAULT_PORT: u16 = 3000;
const MIGRATIONS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");
const PUBLIC_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/public");
const TLS_CERT_CHAIN_ENV: &str = "JOLTR_BASIC_TLS_CERT_CHAIN";
const TLS_PRIVATE_KEY_ENV: &str = "JOLTR_BASIC_TLS_PRIVATE_KEY";

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

    configure_tls_from_env(JoltRServer::new().port(DEFAULT_PORT).cors(cors))?
        .endpoint(endpoints::TemplateEndpoint::new()?)
        .endpoint(endpoints::EchoEndpoint)
        .endpoint(endpoints::ItemEndpoint)
        .start(static_assets_router())
        .await?;

    Ok(())
}

fn configure_tls_from_env(server: JoltRServer) -> io::Result<JoltRServer> {
    configure_tls(
        server,
        path_from_env(TLS_CERT_CHAIN_ENV),
        path_from_env(TLS_PRIVATE_KEY_ENV),
    )
}

fn path_from_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.as_os_str().is_empty())
        .map(PathBuf::from)
}

fn configure_tls(
    server: JoltRServer,
    cert_chain_path: Option<PathBuf>,
    private_key_path: Option<PathBuf>,
) -> io::Result<JoltRServer> {
    match (cert_chain_path, private_key_path) {
        (None, None) => Ok(server),
        (Some(cert_chain_path), Some(private_key_path)) => Ok(server.tls(TlsConfig {
            cert_chain_path,
            private_key_path,
        })),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "set both {TLS_CERT_CHAIN_ENV} and {TLS_PRIVATE_KEY_ENV} to enable TLS, or neither for plain HTTP"
            ),
        )),
    }
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
    use serde_json::Value;

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

    #[tokio::test]
    async fn serving_router_exposes_typed_test_endpoint() {
        let router = JoltRServer::new().build_serving_router(static_assets_router());

        let response = router
            .oneshot(
                AxumRequest::builder()
                    .uri("/api/test/typed")
                    .body(Body::empty())
                    .expect("typed endpoint request builds"),
            )
            .await
            .expect("router responds");

        assert_eq!(response.status(), AxumStatusCode::OK);
        let body = to_bytes(response.into_body(), 1024)
            .await
            .expect("body collects");
        let parsed: Value = serde_json::from_slice(&body).expect("valid JSON body");
        assert_eq!(parsed["contract_version"], 1);
        assert_eq!(parsed["service"], "joltr-basic-example");
        assert_eq!(parsed["ok"], true);
    }

    #[test]
    fn default_server_uses_plain_http_without_tls_paths() {
        let server = configure_tls(JoltRServer::new(), None, None).expect("server configures");

        assert!(server.tls_config.is_none());
    }

    #[test]
    fn server_enables_tls_when_both_paths_are_provided() {
        let server = configure_tls(
            JoltRServer::new(),
            Some(PathBuf::from("cert.pem")),
            Some(PathBuf::from("key.pem")),
        )
        .expect("server configures");
        let tls = server.tls_config.expect("TLS config is present");

        assert_eq!(tls.cert_chain_path, PathBuf::from("cert.pem"));
        assert_eq!(tls.private_key_path, PathBuf::from("key.pem"));
    }

    #[test]
    fn tls_configuration_requires_both_paths() {
        let missing_key =
            match configure_tls(JoltRServer::new(), Some(PathBuf::from("cert.pem")), None) {
                Ok(_) => panic!("missing key should fail"),
                Err(err) => err,
            };
        let missing_cert =
            match configure_tls(JoltRServer::new(), None, Some(PathBuf::from("key.pem"))) {
                Ok(_) => panic!("missing cert should fail"),
                Err(err) => err,
            };

        assert_eq!(missing_key.kind(), io::ErrorKind::InvalidInput);
        assert!(missing_key.to_string().contains(TLS_CERT_CHAIN_ENV));
        assert!(missing_key.to_string().contains(TLS_PRIVATE_KEY_ENV));
        assert_eq!(missing_cert.kind(), io::ErrorKind::InvalidInput);
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
