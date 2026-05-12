use joltr_core::{CorsConfig, JoltRServer, Method};

const DEFAULT_PORT: u16 = 3000;

#[tokio::main]
async fn main() -> std::io::Result<()> {
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
        .start(Default::default())
        .await
}
