use joltr_core::{CorsConfig, JoltRServer, Method};
use joltr_db::{DbConfig, JoltRDb};

mod endpoints;
mod tasks;

const DEFAULT_PORT: u16 = 3000;
const MIGRATIONS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");

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
        .start(Default::default())
        .await?;

    Ok(())
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
