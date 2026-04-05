use anyhow::{Context, Result};
use hyper_util::client::legacy::connect::HttpConnector;
use std::path::PathBuf;
use yup_oauth2::{ApplicationSecret, InstalledFlowAuthenticator, InstalledFlowReturnMethod};

pub type TasksHub = google_tasks1::TasksHub<hyper_rustls::HttpsConnector<HttpConnector>>;

const SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/tasks",
    "https://www.googleapis.com/auth/tasks.readonly",
];

fn config_dir() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("ホームディレクトリが見つかりません")?
        .join("gtasks-cli");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn token_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("token_cache.json"))
}

fn secret_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("client_secret.json"))
}

/// JSON ファイルからクライアント情報をインポート
pub fn import_secret(json_path: &str) -> Result<()> {
    let source = std::fs::read_to_string(json_path)
        .with_context(|| format!("ファイルが見つかりません: {}", json_path))?;
    // JSON として有効か検証
    let _: serde_json::Value = serde_json::from_str(&source)
        .context("JSON の形式が不正です")?;
    let dest = secret_path()?;
    std::fs::write(&dest, &source)?;
    println!("クライアント情報をインポートしました: {}", dest.display());
    Ok(())
}

/// 認証済みの TasksHub を作成
pub async fn build_hub(
) -> Result<TasksHub>
{
    let secret_file = secret_path()?;
    if !secret_file.exists() {
        anyhow::bail!(
            "クライアント情報が未設定です。先に `gtasks auth --client-id <ID> --client-secret <SECRET>` を実行してください。"
        );
    }

    let secret_data = std::fs::read_to_string(&secret_file)?;
    let json: serde_json::Value = serde_json::from_str(&secret_data)?;
    let secret: ApplicationSecret =
        serde_json::from_value(json["installed"].clone())
            .context("client_secret.json の形式が不正です")?;

    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    let auth = InstalledFlowAuthenticator::builder(secret, InstalledFlowReturnMethod::HTTPRedirect)
        .persist_tokens_to_disk(token_path()?)
        .build()
        .await
        .context("OAuth2 認証の初期化に失敗しました")?;

    // トークン取得（初回はブラウザが開く）
    let _token = auth
        .token(SCOPES)
        .await
        .context("トークン取得に失敗しました。ブラウザで認証を完了してください。")?;

    let connector = hyper_rustls::HttpsConnectorBuilder::new()
        .with_native_roots()
        .context("TLS ルート証明書の読み込みに失敗しました")?
        .https_only()
        .enable_http2()
        .build();

    let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
        .build(connector);

    Ok(google_tasks1::TasksHub::new(client, auth))
}
