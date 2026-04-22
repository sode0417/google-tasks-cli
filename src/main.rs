mod auth;
mod commands;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "gtasks", about = "Google Tasks CLI", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// OAuth2 認証を設定する（Google Cloud Console からダウンロードした JSON ファイルを指定）
    Auth {
        /// クライアント情報の JSON ファイルパス
        json_file: String,
    },

    /// タスクリスト一覧を表示
    #[command(name = "lists")]
    ListTasklists,

    /// タスクリストを作成
    #[command(name = "lists-create")]
    CreateTasklist {
        /// タスクリスト名
        title: String,
    },

    /// タスクリストを削除
    #[command(name = "lists-delete")]
    DeleteTasklist {
        /// タスクリスト ID
        tasklist_id: String,
    },

    /// タスク一覧を表示
    List {
        /// タスクリスト ID（省略時はデフォルト）
        #[arg(long, conflicts_with = "all_lists")]
        tasklist: Option<String>,
        /// 期限フィルタ: "today", "YYYY-MM-DD", "YYYY-MM-DD..YYYY-MM-DD"
        #[arg(long)]
        due: Option<String>,
        /// 完了済みタスクも表示する
        #[arg(long, default_value_t = false)]
        show_completed: bool,
        /// JSON 形式で出力する（スクリプト連携用）
        #[arg(long, default_value_t = false)]
        json: bool,
        /// 全タスクリストを横断して取得する
        #[arg(long, default_value_t = false)]
        all_lists: bool,
    },

    /// タスクを作成
    Create {
        /// タスクのタイトル
        title: String,
        /// 期限 (YYYY-MM-DD)
        #[arg(long)]
        due: Option<String>,
        /// メモ
        #[arg(long)]
        notes: Option<String>,
        /// タスクリスト ID
        #[arg(long)]
        tasklist: Option<String>,
    },

    /// タスクを完了にする
    Complete {
        /// タスク ID
        task_id: String,
        /// タスクリスト ID
        #[arg(long)]
        tasklist: Option<String>,
    },

    /// タスクを更新
    Update {
        /// タスク ID
        task_id: String,
        /// 新しいタイトル
        #[arg(long)]
        title: Option<String>,
        /// 新しい期限 (YYYY-MM-DD)
        #[arg(long)]
        due: Option<String>,
        /// 新しいメモ
        #[arg(long)]
        notes: Option<String>,
        /// タスクリスト ID
        #[arg(long)]
        tasklist: Option<String>,
    },

    /// タスクを削除
    Delete {
        /// タスク ID
        task_id: String,
        /// タスクリスト ID
        #[arg(long)]
        tasklist: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Auth { json_file } => {
            auth::import_secret(&json_file)?;
            println!("ブラウザで Google 認証を行います...");
            let _hub = auth::build_hub().await?;
            println!("認証が完了しました。");
        }
        Commands::ListTasklists => {
            let hub = auth::build_hub().await?;
            commands::list_tasklists(&hub).await?;
        }
        Commands::CreateTasklist { title } => {
            let hub = auth::build_hub().await?;
            commands::create_tasklist(&hub, &title).await?;
        }
        Commands::DeleteTasklist { tasklist_id } => {
            let hub = auth::build_hub().await?;
            commands::delete_tasklist(&hub, &tasklist_id).await?;
        }
        Commands::List {
            tasklist,
            due,
            show_completed,
            json,
            all_lists,
        } => {
            let hub = auth::build_hub().await?;
            if json {
                commands::list_tasks_json(
                    &hub,
                    tasklist.as_deref(),
                    due.as_deref(),
                    show_completed,
                    all_lists,
                )
                .await?;
            } else {
                commands::list_tasks(
                    &hub,
                    tasklist.as_deref(),
                    due.as_deref(),
                    show_completed,
                    all_lists,
                )
                .await?;
            }
        }
        Commands::Create {
            title,
            due,
            notes,
            tasklist,
        } => {
            let hub = auth::build_hub().await?;
            commands::create_task(&hub, tasklist.as_deref(), &title, due.as_deref(), notes.as_deref())
                .await?;
        }
        Commands::Complete { task_id, tasklist } => {
            let hub = auth::build_hub().await?;
            commands::complete_task(&hub, tasklist.as_deref(), &task_id).await?;
        }
        Commands::Update {
            task_id,
            title,
            due,
            notes,
            tasklist,
        } => {
            let hub = auth::build_hub().await?;
            commands::update_task(
                &hub,
                tasklist.as_deref(),
                &task_id,
                title.as_deref(),
                due.as_deref(),
                notes.as_deref(),
            )
            .await?;
        }
        Commands::Delete { task_id, tasklist } => {
            let hub = auth::build_hub().await?;
            commands::delete_task(&hub, tasklist.as_deref(), &task_id).await?;
        }
    }

    Ok(())
}
