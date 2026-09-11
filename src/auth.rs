use anyhow::{Context, Result};
use hyper_util::client::legacy::connect::HttpConnector;
use std::future::Future;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;
use yup_oauth2::authenticator_delegate::InstalledFlowDelegate;
use yup_oauth2::{ApplicationSecret, InstalledFlowAuthenticator, InstalledFlowReturnMethod};

pub type TasksHub = google_tasks1::TasksHub<hyper_rustls::HttpsConnector<HttpConnector>>;

const SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/tasks",
    "https://www.googleapis.com/auth/tasks.readonly",
];

/// 非対話実行時にトークン取得へ使ってよい時間の上限。
/// 呼び出し側スクリプト（start-session の get_tasks.py など）が 30 秒で見切るため、
/// それより先に自分から失敗して理由を stderr に残す。
const NONINTERACTIVE_TIMEOUT: Duration = Duration::from_secs(20);

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

/// ブラウザ認可（対話フロー）へ落ちることを許すかどうか
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BrowserFlow {
    /// `gtasks auth` のように、ブラウザ認可の実行そのものが目的の呼び出し
    Allow,
    /// 端末に繋がっている時だけ許す。スクリプトから呼ばれた時は即座に失敗させる
    OnlyWhenInteractive,
}

/// 非対話実行時に使うデリゲート。認可 URL を表示せず、その場でエラーにする。
struct RefuseBrowserFlow;

impl InstalledFlowDelegate for RefuseBrowserFlow {
    fn present_user_url<'a>(
        &'a self,
        _url: &'a str,
        _need_code: bool,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<String, String>> + Send + 'a>> {
        Box::pin(async { Err("非対話実行のためブラウザ認可は行いません".to_string()) })
    }
}

fn reauth_hint(secret_file: &Path) -> String {
    format!(
        "保存済みトークンを更新できませんでした（refresh token の失効が疑われます）。\n\
         端末から次を実行して再認証してください:\n  gtasks auth \"{}\"",
        secret_file.display()
    )
}

/// 認証済みの TasksHub を作成（端末かどうかで対話フローの可否を自動判定する）
pub async fn build_hub() -> Result<TasksHub> {
    build_hub_with(BrowserFlow::OnlyWhenInteractive).await
}

/// 認証済みの TasksHub を作成
pub async fn build_hub_with(flow: BrowserFlow) -> Result<TasksHub> {
    let secret_file = secret_path()?;
    if !secret_file.exists() {
        anyhow::bail!(
            "クライアント情報が未設定です。先に `gtasks auth <client_secret.json>` を実行してください。"
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

    // 端末に繋がっていない状態でブラウザ認可へ落ちると、誰も認可を返せないまま
    // ローカルサーバの待ち受けで永久にブロックする。stdin/stdout がどちらも端末の時だけ許す。
    let interactive = flow == BrowserFlow::Allow
        || (std::io::stdin().is_terminal() && std::io::stdout().is_terminal());

    let return_method = if interactive {
        InstalledFlowReturnMethod::HTTPRedirect
    } else {
        // Interactive 方式は present_user_url のエラーをそのまま伝播する。
        // HTTPRedirect 方式は戻り値を捨ててローカルサーバを待ち続けるため、ここでは使えない。
        InstalledFlowReturnMethod::Interactive
    };

    let mut builder = InstalledFlowAuthenticator::builder(secret, return_method)
        .persist_tokens_to_disk(token_path()?);
    if !interactive {
        builder = builder.flow_delegate(Box::new(RefuseBrowserFlow));
    }

    let auth = builder
        .build()
        .await
        .context("OAuth2 認証の初期化に失敗しました")?;

    // トークン取得（初回・再認証時はブラウザが開く）
    if interactive {
        auth.token(SCOPES)
            .await
            .context("トークン取得に失敗しました。ブラウザで認証を完了してください。")?;
    } else {
        match tokio::time::timeout(NONINTERACTIVE_TIMEOUT, auth.token(SCOPES)).await {
            Ok(Ok(_token)) => {}
            Ok(Err(e)) => return Err(anyhow::Error::new(e).context(reauth_hint(&secret_file))),
            Err(_elapsed) => anyhow::bail!(
                "{}\n（{} 秒で中断しました）",
                reauth_hint(&secret_file),
                NONINTERACTIVE_TIMEOUT.as_secs()
            ),
        }
    }

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
