use crate::auth::TasksHub;
use anyhow::{Context, Result};
use chrono::{NaiveDate, Utc};
use google_tasks1::api::{Task, TaskList};
use std::collections::HashSet;

/// tasks.list の1ページの件数（API の上限は 100、指定しないと 20）
const TASKS_PAGE_SIZE: i32 = 100;
/// tasklists.list の1ページの件数（API の上限は 1000）
const TASKLISTS_PAGE_SIZE: i32 = 1000;

/// タスクリスト一覧を表示
pub async fn list_tasklists(hub: &TasksHub) -> Result<()> {
    let lists = fetch_all_tasklists(hub).await?;

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
    let due_range = due.map(parse_due_filter).transpose()?;
    // 各ページの async move に参照だけを持ち込むため、参照に束ね直す
    let due_range = &due_range;

    collect_pages(|page_token| async move {
        let mut call = hub
            .tasks()
            .list(list_id)
            .show_completed(show_completed)
            .show_hidden(show_completed)
            .max_results(TASKS_PAGE_SIZE);

        if let Some((min, max)) = due_range {
            call = call.due_min(min);
            call = call.due_max(max);
        }
        if let Some(token) = &page_token {
            call = call.page_token(token);
        }

        let (_, result) = call.doit().await.context("タスク取得に失敗")?;
        Ok((result.items.unwrap_or_default(), result.next_page_token))
    })
    .await
}

/// 全タスクリストを取得
async fn fetch_all_tasklists(hub: &TasksHub) -> Result<Vec<TaskList>> {
    collect_pages(|page_token| async move {
        let mut call = hub.tasklists().list().max_results(TASKLISTS_PAGE_SIZE);
        if let Some(token) = &page_token {
            call = call.page_token(token);
        }
        let (_, result) = call.doit().await.context("タスクリスト取得に失敗")?;
        Ok((result.items.unwrap_or_default(), result.next_page_token))
    })
    .await
}

/// nextPageToken が返る限りページをたどり、全件を集める
async fn collect_pages<T, F, Fut>(mut fetch_page: F) -> Result<Vec<T>>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: Future<Output = Result<(Vec<T>, Option<String>)>>,
{
    let mut all = Vec::new();
    let mut seen_tokens = HashSet::new();
    let mut page_token: Option<String> = None;
    loop {
        let (items, next) = fetch_page(page_token.clone()).await?;
        all.extend(items);
        match next {
            Some(next) if !next.is_empty() => {
                if !seen_tokens.insert(next.clone()) {
                    anyhow::bail!("同じ nextPageToken が再び返ったため取得を中断しました");
                }
                page_token = Some(next);
            }
            _ => return Ok(all),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    type Page = Result<(Vec<u32>, Option<String>)>;

    /// ページの列を偽の API として返す（token はページ番号、最後のページだけ token なし）
    fn fake_pages(pages: Vec<Vec<u32>>) -> impl FnMut(Option<String>) -> std::future::Ready<Page> {
        move |token| {
            let index = token.map_or(0, |t| t.parse::<usize>().unwrap());
            let next = (index + 1 < pages.len()).then(|| (index + 1).to_string());
            std::future::ready(Ok((pages[index].clone(), next)))
        }
    }

    #[tokio::test]
    async fn collects_all_pages() {
        let pages = vec![
            (0..100).collect(),
            (100..200).collect(),
            (200..261).collect(),
        ];
        let all = collect_pages(fake_pages(pages)).await.unwrap();
        assert_eq!(all.len(), 261);
        assert_eq!(all.first(), Some(&0));
        assert_eq!(all.last(), Some(&260));
    }

    #[tokio::test]
    async fn single_page_without_token() {
        let all = collect_pages(fake_pages(vec![vec![1, 2, 3]]))
            .await
            .unwrap();
        assert_eq!(all, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn empty_token_ends_paging() {
        let all = collect_pages(|_| std::future::ready(Ok((vec![7], Some(String::new())))))
            .await
            .unwrap();
        assert_eq!(all, vec![7]);
    }

    #[tokio::test]
    async fn repeated_token_is_an_error() {
        let result =
            collect_pages(|_| std::future::ready(Ok((vec![1], Some("same".to_string()))))).await;
        let message = result.unwrap_err().to_string();
        assert!(message.contains("nextPageToken"), "{message}");
    }

    #[tokio::test]
    async fn cycling_tokens_are_an_error() {
        let result = collect_pages(|token| {
            let next = match token.as_deref() {
                None | Some("b") => "a",
                Some(_) => "b",
            };
            std::future::ready(Ok((vec![1], Some(next.to_string()))))
        })
        .await;
        let message = result.unwrap_err().to_string();
        assert!(message.contains("nextPageToken"), "{message}");
    }

    #[tokio::test]
    async fn error_on_later_page_is_returned() {
        let result: Result<Vec<u32>> = collect_pages(|token| {
            std::future::ready(match token {
                None => Ok((vec![1], Some("2".to_string()))),
                Some(_) => Err(anyhow::anyhow!("boom")),
            })
        })
        .await;
        assert!(result.is_err());
    }
}
