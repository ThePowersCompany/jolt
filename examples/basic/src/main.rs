use axum::body::Body;
use axum::http::{Request as AxumRequest, StatusCode as AxumStatusCode};
use axum::response::Response as AxumResponse;
use joltr_core::{
    tower::{service_fn, Layer},
    CorsConfig, FileServeLayer, JoltRServer, Method, TlsConfig,
};
use joltr_db::{DbConfig, JoltRDb};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

mod chat;
mod endpoints;
mod tasks;

const DEFAULT_PORT: u16 = 3000;
const MIGRATIONS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");
const PUBLIC_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/public");
const TLS_CERT_CHAIN_ENV: &str = "JOLTR_BASIC_TLS_CERT_CHAIN";
const TLS_PRIVATE_KEY_ENV: &str = "JOLTR_BASIC_TLS_PRIVATE_KEY";
const GENERATE_TYPES_ARG: &str = "--generate-types";
const TYPES_OUT_ENV: &str = "JOLTR_TYPES_OUT";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == GENERATE_TYPES_ARG) {
        let out_path = write_types_file(&resolve_types_out_path())?;
        println!(
            "joltr-basic-example: wrote TypeScript types to {}",
            out_path.display()
        );
        return Ok(());
    }

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
        .start(extra_router())
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

fn resolve_types_out_path() -> PathBuf {
    if let Ok(path) = std::env::var(TYPES_OUT_ENV) {
        return PathBuf::from(path);
    }

    workspace_root().join("types.d.ts")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| panic!("example manifest directory has no workspace root parent"))
        .to_path_buf()
}

fn write_types_file(out_path: &Path) -> io::Result<PathBuf> {
    if let Some(parent) = out_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }

    fs::write(out_path, joltr_types::render())?;
    Ok(out_path.to_path_buf())
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

fn extra_router() -> axum::Router {
    chat::router().merge(static_assets_router())
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
        let router = JoltRServer::new().build_serving_router(extra_router());

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

    #[test]
    fn generated_types_file_includes_typed_test_response_contract() {
        let out_path = tempfile_in_target("types");

        let written_path = write_types_file(&out_path).expect("types file writes");
        let contents = fs::read_to_string(&written_path).expect("types file is readable");

        assert!(contents.contains("export interface TypedTestResponse {"));
        assert!(contents.contains("contract_version: number;"));
        assert!(contents.contains("service: string;"));
        assert!(contents.contains("ok: boolean;"));
        assert!(contents.contains("features: string[];"));

        let _ = fs::remove_file(written_path);
    }

    fn tempfile_in_target(suffix: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);

        workspace_root()
            .join("target")
            .join(format!("joltr-basic-example-{nanos}-{suffix}.d.ts"))
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
