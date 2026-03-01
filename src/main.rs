use reminisce::{run_server, config::Config, telemetry};
use std::env;

#[tokio::main]
pub async fn main() -> std::io::Result<()> {
    // Initialize rustls crypto provider
    rustls::crypto::ring::default_provider().install_default().expect("Failed to install rustls crypto provider");

    let args: Vec<String> = env::args().collect();

    if args.len() != 2 {
        eprintln!("Usage: {} <config_file_path>", args[0]);
        std::process::exit(1);
    }
    
    let config_path = &args[1];
    let config = Config::from_file(config_path).expect("Failed to load config file");

    // Initialize telemetry
    if let Err(e) = telemetry::init_telemetry(&config) {
        eprintln!("Failed to initialize telemetry: {}", e);
    }
    
    run_server(config).await
}
