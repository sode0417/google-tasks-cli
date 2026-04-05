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

/// タスク一覧を表示
pub async fn list_tasks(
    hub: &TasksHub,
    tasklist: Option<&str>,
    due: Option<&str>,
    show_completed: bool,
) -> Result<()> {
    let list_id = default_tasklist_id(tasklist);

    let mut call = hub.tasks().list(&list_id).show_completed(show_completed).show_hidden(false);

    if let Some(due_str) = due {
        let (min, max) = parse_due_filter(due_str)?;
        call = call.due_min(&min);
        call = call.due_max(&max);
    }

    let (_, result) = call.doit().await.context("タスク取得に失敗")?;
    let tasks = result.items.unwrap_or_default();

    if tasks.is_empty() {
        println!("タスクがありません。");
        return Ok(());
    }

    for task in &tasks {
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

/// JSON 形式でタスク一覧を出力（スクリプト連携用）
pub async fn list_tasks_json(
    hub: &TasksHub,
    tasklist: Option<&str>,
    due: Option<&str>,
    show_completed: bool,
) -> Result<()> {
    let list_id = default_tasklist_id(tasklist);

    let mut call = hub.tasks().list(&list_id).show_completed(show_completed).show_hidden(false);

    if let Some(due_str) = due {
        let (min, max) = parse_due_filter(due_str)?;
        call = call.due_min(&min);
        call = call.due_max(&max);
    }

    let (_, result) = call.doit().await.context("タスク取得に失敗")?;
    let tasks = result.items.unwrap_or_default();

    let output: Vec<serde_json::Value> = tasks
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "title": t.title,
                "status": t.status,
                "due": t.due,
                "notes": t.notes,
                "completed": t.completed,
            })
        })
        .collect();

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// due フィルタ文字列をパース
fn parse_due_filter(due_str: &str) -> Result<(String, String)> {
    match due_str {
        "today" => {
            let today = chrono::Local::now().date_naive();
            let min = format!("{}T00:00:00.000Z", today);
            let max = format!("{}T23:59:59.999Z", today);
            Ok((min, max))
        }
        s if s.contains("..") => {
            let parts: Vec<&str> = s.split("..").collect();
            if parts.len() != 2 {
                anyhow::bail!("日付範囲の形式が不正です (YYYY-MM-DD..YYYY-MM-DD)");
            }
            let start = NaiveDate::parse_from_str(parts[0], "%Y-%m-%d")
                .context("開始日の形式が不正です")?;
            let end =
                NaiveDate::parse_from_str(parts[1], "%Y-%m-%d").context("終了日の形式が不正です")?;
            Ok((
                format!("{}T00:00:00.000Z", start),
                format!("{}T23:59:59.999Z", end),
            ))
        }
        s => {
            let date =
                NaiveDate::parse_from_str(s, "%Y-%m-%d").context("日付の形式が不正です (YYYY-MM-DD)")?;
            let min = format!("{}T00:00:00.000Z", date);
            let max = format!("{}T23:59:59.999Z", date);
            Ok((min, max))
        }
    }
}
