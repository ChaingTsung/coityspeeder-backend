mod core;
mod detector;

use axum::{routing::post, Json, Router};
use clap::{Parser, Subcommand};
use serde::Deserialize;
use tracing::{info, error};
use tracing_subscriber::{fmt, EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Server { #[arg(short, long, default_value = "0.0.0.0:3000")] bind: String },
    Test { #[arg(short, long)] target: String, #[arg(long)] sub_url: Option<String>, #[arg(long)] is_file: bool },
}

#[derive(Deserialize)]
struct TestRequest { 
    target: String, 
    sub_url: Option<String>,
    is_file: Option<bool>
}

#[tokio::main]
async fn main() {
    let file_appender = tracing_appender::rolling::daily("logs", "coityspeeder.log");
    let (non_blocking_appender, _guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::registry()
        .with(EnvFilter::new(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into())))
        .with(fmt::layer().with_writer(std::io::stderr)) 
        .with(fmt::layer().with_writer(non_blocking_appender).with_ansi(false))
        .init();

    let cli = Cli::parse();
    match &cli.command {
        Commands::Server { bind } => {
            let app = Router::new().route("/api/test", post(api_handler));
            let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
            info!("🚀 CoitySpeeder API Server running on {}", bind);
            if let Err(e) = axum::serve(listener, app).await {
                error!("API 服务器运行异常: {}", e);
            }
        }
        Commands::Test { target, sub_url, is_file } => {
            info!("接收到终端测速任务...");
            let result = core::execute_test(target, sub_url.clone(), *is_file, 55000).await;
            println!("{}", serde_json::to_string(&result).unwrap());
            info!("终端测速任务执行完毕。");
        }
    }
}

async fn api_handler(Json(payload): Json<TestRequest>) -> Json<detector::TestResult> {
    let is_file = payload.is_file.unwrap_or(false);
    let log_msg = if is_file { "本地配置文件" } else { &payload.target };
    info!("接收到 Web 测速任务, 目标: {}", log_msg);
    
    let result = core::execute_test(&payload.target, payload.sub_url, is_file, 55000).await;
    info!("Web 测速任务执行完毕, 节点: {}", result.node_name);
    Json(result)
}