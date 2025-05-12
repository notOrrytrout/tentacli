use anyhow::Result as AnyResult;
use tentacli::{Client, CreateOptions, RunOptions};
use serde::Deserialize;
use std::fs;

#[derive(Deserialize)]
struct AppConfig {
    username:     String,
    password:     String,
    host:         String,
    port:         u16,
    realm:        String,
    character:    String,
    log_file:     Option<String>,
    login_delay:  Option<u64>,
}

#[tokio::main]
async fn main() -> AnyResult<()> {
    // Load YAML config
    let s = fs::read_to_string("Config.yml")?;
    let cfg: AppConfig = serde_yaml::from_str(&s)?;

    // Run client
    Client::new(CreateOptions {
        data_storage: None,
    })
    .run(RunOptions {
        external_features: vec![],
        account: &cfg.username,
        password: &cfg.password,
        host: &cfg.host,
        port: cfg.port,
        realm: &cfg.realm,
        character: &cfg.character,
        log_file: cfg.log_file.clone(),
        login_delay: cfg.login_delay,
        config_path: "Config.yml",
        dotenv_path: "",
    })
    .await?;

    Ok(())
}
