//! HTTP server, one-second simulation, and the agent worker.

use ai_town_kernel::{AGENT_BASE, ENGINE};
use ai_town_server::{principal, serve, Provider, Town};
use h5i_app::http::HmacAuth;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let secret = std::env::var("H5I_APP_SECRET").map_err(|_| "H5I_APP_SECRET is not set")?;
    let auth = Arc::new(HmacAuth::<Town>::new(secret.clone(), principal));
    if let Ok(spec) = std::env::var("H5I_APP_ISSUE") {
        let (org, user) = spec.split_once(':').ok_or("H5I_APP_ISSUE=org:user")?;
        let user: u64 = user.parse()?;
        if user == ENGINE || user >= AGENT_BASE {
            return Err("refusing to issue that user".into());
        }
        println!("{}", auth.issue(org.parse()?, user, 86_400));
        return Ok(());
    }

    let url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is not set")?;
    let addr = std::env::var("LISTEN").unwrap_or_else(|_| "127.0.0.1:8080".into());
    serve(&url, &secret, &addr, Provider::from_env()?).await?;
    Ok(())
}
