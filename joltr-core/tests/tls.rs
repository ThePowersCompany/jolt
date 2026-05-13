use std::io;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use joltr_core::{JoltRServer, TlsConfig};

#[tokio::test]
async fn tls_start_rejects_missing_certificate_file() {
    let dir = unique_tls_fixture_dir("missing-cert");
    std::fs::create_dir_all(&dir).expect("create tls fixture dir");

    let err = start_with_tls(TlsConfig {
        cert_chain_path: dir.join("missing-cert.pem"),
        private_key_path: dir.join("missing-key.pem"),
    })
    .await;

    let _ = std::fs::remove_dir_all(dir);
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
    assert!(
        err.to_string().contains("TLS certificate chain path"),
        "error should identify the certificate path: {err}"
    );
}

#[tokio::test]
async fn tls_start_rejects_missing_private_key_file() {
    let dir = unique_tls_fixture_dir("missing-key");
    std::fs::create_dir_all(&dir).expect("create tls fixture dir");
    let cert_path = dir.join("cert.pem");
    std::fs::write(&cert_path, "not parsed before key readability check")
        .expect("write cert placeholder");

    let err = start_with_tls(TlsConfig {
        cert_chain_path: cert_path,
        private_key_path: dir.join("missing-key.pem"),
    })
    .await;

    let _ = std::fs::remove_dir_all(dir);
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
    assert!(
        err.to_string().contains("TLS private key path"),
        "error should identify the private key path: {err}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn tls_start_rejects_unreadable_private_key_file() {
    use std::os::unix::fs::PermissionsExt;

    let dir = unique_tls_fixture_dir("unreadable-key");
    std::fs::create_dir_all(&dir).expect("create tls fixture dir");
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    std::fs::write(&cert_path, "not parsed before key readability check")
        .expect("write cert placeholder");
    std::fs::write(&key_path, "not readable").expect("write key placeholder");
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o000))
        .expect("make key unreadable");

    let err = start_with_tls(TlsConfig {
        cert_chain_path: cert_path,
        private_key_path: key_path.clone(),
    })
    .await;

    let _ = std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600));
    let _ = std::fs::remove_dir_all(dir);
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    assert!(
        err.to_string().contains("TLS private key path"),
        "error should identify the private key path: {err}"
    );
}

#[tokio::test]
async fn tls_start_rejects_malformed_pem_files() {
    let dir = unique_tls_fixture_dir("malformed");
    std::fs::create_dir_all(&dir).expect("create tls fixture dir");
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    std::fs::write(&cert_path, "definitely not a certificate").expect("write malformed cert");
    std::fs::write(&key_path, "definitely not a private key").expect("write malformed key");

    let err = start_with_tls(TlsConfig {
        cert_chain_path: cert_path,
        private_key_path: key_path,
    })
    .await;

    let _ = std::fs::remove_dir_all(dir);
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    assert!(
        err.to_string()
            .contains("invalid TLS certificate/key configuration"),
        "error should identify malformed TLS material: {err}"
    );
}

async fn start_with_tls(config: TlsConfig) -> io::Error {
    JoltRServer::new()
        .port(0)
        .tls(config)
        .start(Router::new())
        .await
        .expect_err("invalid TLS config should fail before serving")
}

fn unique_tls_fixture_dir(suffix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "joltr-core-tls-{suffix}-{}-{nanos}",
        std::process::id()
    ))
}
