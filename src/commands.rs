use crate::auth::TasksHub;
use anyhow::{Context, Result};
use chrono::{NaiveDate, Utc};
use google_tasks1::api::{Task, TaskList};

/// タスクリスト一覧を表示
pub async fn list_tasklists(hub: &TasksHub) -> Result<()> {
    let (_, result) = hub.tasklists().list().doit().await.context("タスクリスト取得に失敗")?;
    let lists = result.items.unwrap_or_default();

    if lists.is_empty() {
        println!("タスクリストがありません。");
        return Ok(());
    }

    for list in &lists {
        println!(
            "  {} (id: {})",
            list.title.as_deref().unwrap_or("(無題)"),
            list.id.as_deref().unwrap_or("?")
        );
    }
    Ok(())
}

/// タスクリストを作成
pub async fn create_tasklist(hub: &TasksHub, title: &str) -> Result<()> {
    let mut tasklist = TaskList::default();
    tasklist.title = Some(title.to_string());

    let (_, created) = hub
        .tasklists()
        .insert(tasklist)
        .doit()
        .await
        .context("タスクリスト作成に失敗")?;

    println!(
        "タスクリストを作成しました: {} [id: {}]",
        created.title.as_deref().unwrap_or("?"),
        created.id.as_deref().unwrap_or("?")
    );
    Ok(())
}

/// タスクリストを削除
pub async fn delete_tasklist(hub: &TasksHub, tasklist_id: &str) -> Result<()> {
    hub.tasklists()
        .delete(tasklist_id)
        .doit()
        .await
        .context("タスクリスト削除に失敗")?;

    println!("タスクリストを削除しました: {}", tasklist_id);
    Ok(())
}

/// デフォルトのタスクリスト ID を取得
fn default_tasklist_id(tasklist: Option<&str>) -> String {
    tasklist.unwrap_or("@default").to_string()
}

/// 指定リストのタスクを取得
async fn fetch_tasks(
    hub: &TasksHub,
    list_id: &str,
    due: Option<&str>,
    show_completed: bool,
) -> Result<Vec<Task>> {
    let mut call = hub
        .tasks()
        .list(list_id)
        .show_completed(show_completed)
        .show_hidden(show_completed);

    if let Some(due_str) = due {
        let (min, max) = parse_due_filter(due_str)?;
        call = call.due_min(&min);
        call = call.due_max(&max);
    }

    let (_, result) = call.doit().await.context("タスク取得に失敗")?;
    Ok(result.items.unwrap_or_default())
}

/// 全タスクリストを取得
async fn fetch_all_tasklists(hub: &TasksHub) -> Result<Vec<TaskList>> {
    let (_, result) = hub
        .tasklists()
        .list()
        .doit()
        .await
        .context("タスクリスト取得に失敗")?;
    Ok(result.items.unwrap_or_default())
}

fn print_task(task: &Task) {
    let status_mark = match task.status.as_deref() {
        Some("completed") => "[x]",
        _ => "[ ]",
    };
    let due_str = task
        .due
        .as_ref()
        .and_then(|d: &String| d.get(..10))
        .unwrap_or("");
    let title = task.title.as_deref().unwrap_or("(無題)");
    let id = task.id.as_deref().unwrap_or("?");

    println!("  {} {} (due: {}) [id: {}]", status_mark, title, due_str, id);

    if let Some(ref notes) = task.notes {
        if !notes.is_empty() {
            println!("      notes: {}", notes);
        }
    }
}

/// タスク一覧を表示
pub async fn list_tasks(
    hub: &TasksHub,
    tasklist: Option<&str>,
    due: Option<&str>,
    show_completed: bool,
    all_lists: bool,
) -> Result<()> {
    if all_lists {
        let lists = fetch_all_tasklists(hub).await?;
        let mut total = 0usize;
        for list in &lists {
            let list_id = list.id.as_deref().unwrap_or("");
            let list_name = list.title.as_deref().unwrap_or("(無題)");
            let tasks = fetch_tasks(hub, list_id, due, show_completed).await?;
            if tasks.is_empty() {
                continue;
            }
            println!("## {}", list_name);
            for task in &tasks {
                print_task(task);
            }
            println!();
            total += tasks.len();
        }
        if total == 0 {
            println!("タスクがありません。");
        }
        return Ok(());
    }

    let list_id = default_tasklist_id(tasklist);
    let tasks = fetch_tasks(hub, &list_id, due, show_completed).await?;

    if tasks.is_empty() {
        println!("タスクがありません。");
        return Ok(());
    }

    for task in &tasks {
        print_task(task);
    }
    Ok(())
}

/// タスクを作成
pub async fn create_task(
    hub: &TasksHub,
    tasklist: Option<&str>,
    title: &str,
    due: Option<&str>,
    notes: Option<&str>,
) -> Result<()> {
    let list_id = default_tasklist_id(tasklist);

    let mut task = Task::default();
    task.title = Some(title.to_string());

    if let Some(due_str) = due {
        let date = NaiveDate::parse_from_str(due_str, "%Y-%m-%d")
            .context("日付の形式が不正です (YYYY-MM-DD)")?;
        task.due = Some(format!("{}T00:00:00.000Z", date));
    }

    if let Some(n) = notes {
        task.notes = Some(n.to_string());
    }

    let (_, created) = hub
        .tasks()
        .insert(task, &list_id)
        .doit()
        .await
        .context("タスク作成に失敗")?;

    println!(
        "タスクを作成しました: {} [id: {}]",
        created.title.as_deref().unwrap_or("?"),
        created.id.as_deref().unwrap_or("?")
    );
    Ok(())
}

/// タスクを完了にする
pub async fn complete_task(hub: &TasksHub, tasklist: Option<&str>, task_id: &str) -> Result<()> {
    let list_id = default_tasklist_id(tasklist);

    let (_, mut task) = hub
        .tasks()
        .get(&list_id, task_id)
        .doit()
        .await
        .context("タスク取得に失敗")?;

    task.status = Some("completed".to_string());
    task.completed = Some(Utc::now().to_rfc3339());

    hub.tasks()
        .update(task, &list_id, task_id)
        .doit()
        .await
        .context("タスク完了に失敗")?;

    println!("タスクを完了にしました: {}", task_id);
    Ok(())
}

/// タスクを更新
pub async fn update_task(
    hub: &TasksHub,
    tasklist: Option<&str>,
    task_id: &str,
    title: Option<&str>,
    due: Option<&str>,
    notes: Option<&str>,
) -> Result<()> {
    let list_id = default_tasklist_id(tasklist);

    let (_, mut task) = hub
        .tasks()
        .get(&list_id, task_id)
        .doit()
        .await
        .context("タスク取得に失敗")?;

    if let Some(t) = title {
        task.title = Some(t.to_string());
    }
    if let Some(d) = due {
        let date = NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .context("日付の形式が不正です (YYYY-MM-DD)")?;
        task.due = Some(format!("{}T00:00:00.000Z", date));
    }
    if let Some(n) = notes {
        task.notes = Some(n.to_string());
    }

    hub.tasks()
        .update(task, &list_id, task_id)
        .doit()
        .await
        .context("タスク更新に失敗")?;

    println!("タスクを更新しました: {}", task_id);
    Ok(())
}

/// タスクを削除
pub async fn delete_task(hub: &TasksHub, tasklist: Option<&str>, task_id: &str) -> Result<()> {
    let list_id = default_tasklist_id(tasklist);

    hub.tasks()
        .delete(&list_id, task_id)
        .doit()
        .await
        .context("タスク削除に失敗")?;

    println!("タスクを削除しました: {}", task_id);
    Ok(())
}

fn task_to_json(task: &Task, list_name: Option<&str>) -> serde_json::Value {
    let mut obj = serde_json::json!({
        "id": task.id,
        "title": task.title,
        "status": task.status,
        "due": task.due,
        "notes": task.notes,
        "completed": task.completed,
    });
    if let Some(name) = list_name {
        obj["list_name"] = serde_json::json!(name);
    }
    obj
}

/// JSON 形式でタスク一覧を出力（スクリプト連携用）
pub async fn list_tasks_json(
    hub: &TasksHub,
    tasklist: Option<&str>,
    due: Option<&str>,
    show_completed: bool,
    all_lists: bool,
) -> Result<()> {
    let output: Vec<serde_json::Value> = if all_lists {
        let lists = fetch_all_tasklists(hub).await?;
        let mut all = Vec::new();
        for list in &lists {
            let list_id = list.id.as_deref().unwrap_or("");
            let list_name = list.title.as_deref().unwrap_or("(無題)");
            let tasks = fetch_tasks(hub, list_id, due, show_completed).await?;
            for task in &tasks {
                all.push(task_to_json(task, Some(list_name)));
            }
        }
        all
    } else {
        let list_id = default_tasklist_id(tasklist);
        let tasks = fetch_tasks(hub, &list_id, due, show_completed).await?;
        tasks.iter().map(|t| task_to_json(t, None)).collect()
    };

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// due フィルタ文字列をパース
///
/// Google Tasks API の `dueMax` は exclusive upper bound として扱われるため、
/// 終了日の翌日 00:00:00.000Z を dueMax として指定する。
fn parse_due_filter(due_str: &str) -> Result<(String, String)> {
    let (start, end) = match due_str {
        "today" => {
            let today = chrono::Local::now().date_naive();
            (today, today)
        }
        s if s.contains("..") => {
            let parts: Vec<&str> = s.split("..").collect();
            if parts.len() != 2 {
                anyhow::bail!("日付範囲の形式が不正です (YYYY-MM-DD..YYYY-MM-DD)");
            }
            let start = NaiveDate::parse_from_str(parts[0], "%Y-%m-%d")
                .context("開始日の形式が不正です")?;
            let end = NaiveDate::parse_from_str(parts[1], "%Y-%m-%d")
                .context("終了日の形式が不正です")?;
            if end < start {
                anyhow::bail!("終了日が開始日より前です");
            }
            (start, end)
        }
        s => {
            let date = NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .context("日付の形式が不正です (YYYY-MM-DD)")?;
            (date, date)
        }
    };

    let end_exclusive = end
        .succ_opt()
        .context("終了日の翌日を計算できません")?;
    Ok((
        format!("{}T00:00:00.000Z", start),
        format!("{}T00:00:00.000Z", end_exclusive),
    ))
}
