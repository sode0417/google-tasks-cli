use anyhow::{Context, Result};
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use hyper_util::client::legacy::connect::HttpConnector;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use yup_oauth2::authenticator_delegate::InstalledFlowDelegate;
use yup_oauth2::{ApplicationSecret, InstalledFlowAuthenticator, InstalledFlowReturnMethod};

pub type TasksHub = google_tasks1::TasksHub<hyper_rustls::HttpsConnector<HttpConnector>>;

const SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/tasks",
    "https://www.googleapis.com/auth/tasks.readonly",
];

/// 非対話時にトークン取得を打ち切るまでの秒数。
///
/// yup-oauth2 はリフレッシュに失敗すると対話フロー（ブラウザ待ち）へ無条件に
/// フォールバックし、その待ち受けにはタイムアウトが無い。launchd や `claude -p`
/// のような非対話環境ではブラウザ認証が完了しないため、そのままでは永久にハングする。
const DEFAULT_AUTH_TIMEOUT_SECS: u64 = 30;

fn auth_timeout() -> Duration {
    let secs = std::env::var("GTASKS_AUTH_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_AUTH_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

/// 「再認証が必要」を呼び出し側（終了コード）に伝えるためのエラー。
///
/// トークンのリフレッシュに失敗すると yup-oauth2 は対話フローへ落ちる。
/// 非対話環境ではそれが永久待ちになるため、対話フローの開始を検知した時点で
/// このエラーにして打ち切る。
#[derive(Debug)]
pub struct ReauthRequired {
    /// 期限切れのトークンが残っていたか（＝リフレッシュに失敗したのか、未認証なのか）
    pub had_token: bool,
    /// 再認証に使うクライアント情報の場所（環境ごとに異なるので実際の値を出す）
    pub secret_path: String,
}

impl std::fmt::Display for ReauthRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cause = if self.had_token {
            "アクセストークンが期限切れで、リフレッシュにも失敗しました"
        } else {
            "有効なトークンがありません（未認証です）"
        };
        write!(
            f,
            "再認証が必要です（{}）。\n\
             非対話モードのため、ブラウザ認証の待ち受けには入らずに終了します。\n\
             対話できる環境で次を実行してください:\n\
             \n    gtasks auth \"{}\"\n",
            cause, self.secret_path
        )
    }
}

impl std::error::Error for ReauthRequired {}

/// 対話フロー（ブラウザ認証）の開始を検知するためのデリゲート。
///
/// yup-oauth2 はリフレッシュに失敗すると対話フローへフォールバックし、
/// その直前に `present_user_url` を呼ぶ。ここを掴むことで
/// 「リフレッシュが失敗して対話へ落ちた」ことを確実に判定できる。
struct InteractiveFlowDetector {
    notify: Arc<Notify>,
}

impl InstalledFlowDelegate for InteractiveFlowDetector {
    fn present_user_url<'a>(
        &'a self,
        url: &'a str,
        _need_code: bool,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<String, String>> + Send + 'a>> {
        let notify = self.notify.clone();
        Box::pin(async move {
            // 認証 URL は stdout を汚さないよう stderr に出す
            eprintln!("再認証が必要です。ブラウザで次の URL を開いてください:\n{}", url);
            notify.notify_one();
            Err("非対話モードのため対話認証は行いません".to_string())
        })
    }
}

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

/// 認証済みの TasksHub を作成する（非対話）。
///
/// トークンが期限切れでリフレッシュにも失敗した場合、yup-oauth2 は対話フロー
/// （ブラウザ待ち）へ落ちて永久にハングする。ここではタイムアウトを掛けて、
/// ハングではなく「再認証が必要」という明示的なエラーで終わらせる。
pub async fn build_hub() -> Result<TasksHub> {
    build_hub_inner(false).await
}

/// 認証済みの TasksHub を作成する（対話を許可）。
///
/// `gtasks auth` 専用。ブラウザでの認証完了を待つため、タイムアウトを掛けない。
pub async fn build_hub_interactive() -> Result<TasksHub> {
    build_hub_inner(true).await
}

async fn build_hub_inner(interactive: bool) -> Result<TasksHub>
{
    let secret_file = secret_path()?;
    if !secret_file.exists() {
        anyhow::bail!(
            "クライアント情報が未設定です。先に `gtasks auth <client_secret.json のパス>` を実行してください。\n\
             JSON は Google Cloud Console からダウンロードできます。"
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

    let notify = Arc::new(Notify::new());
    let auth = if interactive {
        InstalledFlowAuthenticator::builder(secret, InstalledFlowReturnMethod::HTTPRedirect)
            .persist_tokens_to_disk(token_path()?)
            .build()
            .await
    } else {
        // 非対話時は対話フローの開始を検知できるデリゲートを差す
        InstalledFlowAuthenticator::builder(secret, InstalledFlowReturnMethod::HTTPRedirect)
            .persist_tokens_to_disk(token_path()?)
            .flow_delegate(Box::new(InteractiveFlowDetector {
                notify: notify.clone(),
            }))
            .build()
            .await
    }
    .context("OAuth2 認証の初期化に失敗しました")?;

    // トークン取得（初回はブラウザが開く）
    if interactive {
        auth.token(SCOPES)
            .await
            .context("トークン取得に失敗しました。ブラウザで認証を完了してください。")?;
    } else {
        let timeout = auth_timeout();
        tokio::select! {
            result = auth.token(SCOPES) => {
                result.context("トークン取得に失敗しました")?;
            }
            // 対話フローに落ちた = リフレッシュに失敗した。待たずに打ち切る
            _ = notify.notified() => {
                let had_token = read_token_status().map(|v| !v.is_empty()).unwrap_or(false);
                return Err(anyhow::Error::new(ReauthRequired {
                    had_token,
                    secret_path: secret_file.display().to_string(),
                }));
            }
            // 対話フロー以外の理由で返ってこない場合（ネットワーク等）の保険
            _ = tokio::time::sleep(timeout) => {
                anyhow::bail!(
                    "トークンの取得が {} 秒で完了しませんでした。\n\
                     ネットワークまたは Google の認証サーバに到達できていない可能性があります。\n\
                     トークンの状態は `gtasks token-status` で確認できます。",
                    timeout.as_secs()
                );
            }
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

/// トークンキャッシュ1エントリの状態（⛔ トークンの値そのものは保持しない）
pub struct TokenStatus {
    pub scopes: Vec<String>,
    pub has_refresh_token: bool,
    pub expires_at: Option<DateTime<Local>>,
}

impl TokenStatus {
    /// yup-oauth2 と同じ判定（期限の1分前から期限切れ扱い）
    pub fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(exp) => exp - chrono::Duration::minutes(1) <= Local::now(),
            None => false,
        }
    }
}

/// yup-oauth2 が書く `expires_at`（time クレートの配列表現）を解釈する。
/// `[year, ordinal_day, hour, minute, second, nanosecond, offset_h, offset_m, offset_s]`
fn parse_expires_at(v: &serde_json::Value) -> Option<DateTime<Local>> {
    let a = v.as_array()?;
    if a.len() < 5 {
        return None;
    }
    let num = |i: usize| -> Option<i64> { a.get(i).and_then(|x| x.as_i64()) };
    let (year, ordinal) = (num(0)? as i32, num(1)? as u32);
    let (hour, min, sec) = (num(2)? as u32, num(3)? as u32, num(4)? as u32);
    let offset_secs =
        num(6).unwrap_or(0) * 3600 + num(7).unwrap_or(0) * 60 + num(8).unwrap_or(0);

    let naive = NaiveDate::from_yo_opt(year, ordinal)?.and_hms_opt(hour, min, sec)?;
    // 記録されている時刻は offset 付きなので、offset を引いて UTC に直す
    let utc = Utc
        .from_utc_datetime(&naive)
        .checked_sub_signed(chrono::Duration::seconds(offset_secs))?;
    Some(utc.with_timezone(&Local))
}

/// トークンキャッシュを読む（⛔ ネットワークにも対話にも入らない）。
/// キャッシュが存在しない場合は空の Vec を返す。
pub fn read_token_status() -> Result<Vec<TokenStatus>> {
    let path = token_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = std::fs::read_to_string(&path)
        .with_context(|| format!("トークンキャッシュを読めません: {}", path.display()))?;
    let json: serde_json::Value =
        serde_json::from_str(&data).context("トークンキャッシュの形式が不正です")?;
    let entries = json
        .as_array()
        .context("トークンキャッシュの形式が想定と異なります（配列ではありません）")?;

    Ok(entries
        .iter()
        .map(|e| {
            let token = &e["token"];
            TokenStatus {
                scopes: e["scopes"]
                    .as_array()
                    .map(|s| {
                        s.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                has_refresh_token: token["refresh_token"].is_string(),
                expires_at: parse_expires_at(&token["expires_at"]),
            }
        })
        .collect())
}

/// トークンの状態を表示する。有効なトークンが1つでもあれば true を返す。
/// ⛔ トークンの値は一切表示しない（有無と期限だけ）。
pub fn print_token_status() -> Result<bool> {
    let path = token_path()?;
    let statuses = read_token_status()?;

    if statuses.is_empty() {
        println!("トークンキャッシュがありません: {}", path.display());
        println!("`gtasks auth <client_secret.json>` を実行して認証してください。");
        return Ok(false);
    }

    let mut any_valid = false;
    for (i, st) in statuses.iter().enumerate() {
        if statuses.len() > 1 {
            println!("[エントリ {}]", i + 1);
        }
        println!("  スコープ: {}", st.scopes.join(", "));
        println!(
            "  refresh_token: {}",
            if st.has_refresh_token { "あり" } else { "なし" }
        );
        match st.expires_at {
            Some(exp) => {
                let remaining = exp - Local::now();
                let expired = st.is_expired();
                println!("  有効期限: {}", exp.format("%Y-%m-%d %H:%M:%S"));
                if expired {
                    println!(
                        "  状態: 期限切れ（{} 分前に失効）",
                        -remaining.num_minutes()
                    );
                } else {
                    println!("  状態: 有効（残り {} 分）", remaining.num_minutes());
                    any_valid = true;
                }
            }
            None => println!("  有効期限: 不明（記録がありません）"),
        }
    }

    if !any_valid {
        println!();
        println!("有効なトークンがありません。次回のコマンド実行時にリフレッシュが試みられます。");
        println!("リフレッシュに失敗すると再認証が必要です（非対話環境ではエラーで終了します）。");
    }
    Ok(any_valid)
}
