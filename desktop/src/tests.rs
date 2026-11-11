use super::*;
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

#[component]
fn TestWorkspace() -> impl IntoElement {
    set_tokens(nsh_base::tokens_of(Some(include_str!("../theme.json"))).unwrap());
    let directory = tempfile::tempdir().unwrap();
    let api = api::Api::isolated(directory.path().to_path_buf()).unwrap();
    let store = store::Store::with_api(api);
    provide_context(store);
    cleanup(move || drop(directory));
    effect(move || store.reload());
    reader::workspace()
}

fn start(width: f32) -> Headless {
    Headless::start(EngineConfig {
        viewport: Size::new(px(width), px(800.0)),
        ..EngineConfig::new(Root::of::<TestWorkspace>())
    })
    .unwrap()
}

#[test]
fn native_reader_can_add_local_subscription_and_open_connection_settings() -> Result<(), String> {
    let mut app = start(1200.0);
    app.wait_for("这里还没有文章")?;
    app.press("label:添加订阅")?;
    app.type_text("label:Feed 地址", "https://example.com/feed.xml")?;
    app.press("role:button[添加]")?;
    app.wait_for("已添加订阅，正在后台获取文章。")?;
    app.press("服务器与同步")?;
    app.wait_for("可选的远端连接")?;
    app.press("添加服务器…")?;
    app.wait_for("连接远端 Raindrop")?;
    app.type_text("label:HTTPS 服务地址", "https://reader.example.com")?;
    app.type_text("label:用户名或邮箱", "reader")?;
    app.type_text("label:密码", "secret")?;
    app.press("role:button[取消]")?;
    app.press("完成")?;
    app.wait_for("搜索文章")
}

#[test]
fn narrow_reader_keeps_navigation_and_editing_shortcuts_separate() -> Result<(), String> {
    let mut app = start(680.0);
    app.wait_for("这里还没有文章")?;
    app.type_text("label:搜索文章", "jksm")?;
    assert_eq!(app.screen().input_text("label:搜索文章")?, Some("jksm"));
    app.submit("label:搜索文章")?;
    app.press("role:button[订阅]")?;
    app.wait_for("label:添加订阅")?;
    app.press("未读")?;
    app.wait_for("未读")?;
    Ok(())
}

#[test]
fn language_switch_is_live_and_keeps_search_and_theme_selection() -> Result<(), String> {
    let mut app = start(1200.0);
    app.wait_for("这里还没有文章")?;
    app.type_text("label:搜索文章", "native reader")?;
    app.press("服务器与同步")?;
    app.wait_for("外观与语言")?;
    app.press("English")?;
    app.wait_for("Appearance & language")?;
    app.wait_for("Optional server connections")?;
    app.press("role:button[Dark]")?;
    app.wait_until(
        "dark theme saved",
        std::time::Duration::from_secs(5),
        |screen| screen.selected("role:button[Dark]").unwrap_or(false),
    )?;
    assert!(app.screen().selected("role:button[Dark]")?);
    app.press("Done")?;
    app.wait_for("No articles yet")?;
    assert_eq!(
        app.screen().input_text("label:Search articles")?,
        Some("native reader")
    );
    app.press("Settings & sync")?;
    assert!(app.screen().selected("role:button[Dark]")?);
    app.press("role:button[System]")?;
    app.wait_until(
        "system theme saved",
        std::time::Duration::from_secs(5),
        |screen| screen.selected("role:button[System]").unwrap_or(false),
    )?;
    app.press("简体中文")?;
    app.wait_for("外观与语言")?;
    app.press("完成")?;
    assert_eq!(
        app.screen().input_text("label:搜索文章")?,
        Some("native reader")
    );
    Ok(())
}

#[component]
fn DamagedSyncWorkspace() -> impl IntoElement {
    use sea_orm::ConnectionTrait;
    set_tokens(nsh_base::tokens_of(Some(include_str!("../theme.json"))).unwrap());
    let directory = tempfile::tempdir().unwrap();
    let api = api::Api::isolated(directory.path().to_path_buf()).unwrap();
    let core = api.local_core().unwrap();
    sync::profiles(&api).unwrap();
    core.block_on(
        core.database.execute_unprepared(
            "INSERT INTO desktop_sync_state(id, document) VALUES(1, 'damaged')",
        ),
    )
    .unwrap();
    let store = store::Store::with_api(api);
    provide_context(store);
    cleanup(move || drop(directory));
    effect(move || store.reload());
    reader::workspace()
}

#[test]
fn broken_sync_journal_does_not_block_local_reading_or_enable_remote_writes() -> Result<(), String>
{
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(px(1200.0), px(800.0)),
        ..EngineConfig::new(Root::of::<DamagedSyncWorkspace>())
    })
    .unwrap();
    app.wait_for("这里还没有文章")?;
    app.wait_for("同步记录无法读取，已禁用远端操作；本地阅读仍可使用。请保留数据库以便恢复。")?;
    app.press("服务器与同步")?;
    assert!(!app.screen().enabled("添加服务器…")?);
    Ok(())
}

#[component]
fn DamagedPreferencesWorkspace() -> impl IntoElement {
    use sea_orm::ConnectionTrait;
    set_tokens(nsh_base::tokens_of(Some(include_str!("../theme.json"))).unwrap());
    let directory = tempfile::tempdir().unwrap();
    let api = api::Api::isolated(directory.path().to_path_buf()).unwrap();
    let core = api.local_core().unwrap();
    preferences::Preferences::load(&core).unwrap();
    core.block_on(core.database.execute_unprepared("INSERT INTO desktop_preferences(id, schema_version, language, theme) VALUES(1, 2, 'future', 'future')")).unwrap();
    let store = store::Store::with_api(api);
    provide_context(store);
    cleanup(move || drop(directory));
    effect(move || store.reload());
    reader::workspace()
}

#[test]
fn damaged_preferences_preserve_local_reader_and_do_not_allow_overwrite() -> Result<(), String> {
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(px(1200.0), px(800.0)),
        ..EngineConfig::new(Root::of::<DamagedPreferencesWorkspace>())
    })
    .unwrap();
    app.wait_for("这里还没有文章")?;
    app.press("服务器与同步")?;
    app.wait_for("无法读取外观设置，已使用默认值；原有设置未被覆盖。")?;
    assert!(!app.screen().enabled("English")?);
    assert!(app.screen().enabled("添加服务器…")?);
    app.press("完成")?;
    app.press("label:添加订阅")?;
    app.wait_for("label:Feed 地址")?;
    Ok(())
}

#[component]
fn UserContentTranslationWorkspace() -> impl IntoElement {
    let directory = tempfile::tempdir().unwrap();
    let api = api::Api::isolated(directory.path().to_path_buf()).unwrap();
    let store = store::Store::with_api(api);
    provide_context(store);
    cleanup(move || drop(directory));
    view()
        .child(ui::command("English", move || {
            store.set_preferences(preferences::Preferences {
                language: preferences::Language::English,
                theme: preferences::Theme::System,
            })
        }))
        .child(ui::heading("收藏"))
        .child(ui::caption(move || "收藏"))
}

#[test]
fn translating_interface_labels_does_not_translate_dynamic_user_content() -> Result<(), String> {
    let mut app = Headless::start(EngineConfig::new(
        Root::of::<UserContentTranslationWorkspace>(),
    ))
    .unwrap();
    app.wait_for("English")?;
    app.press("English")?;
    app.wait_for("Starred")?;
    app.wait_for("text:收藏")?;
    Ok(())
}
