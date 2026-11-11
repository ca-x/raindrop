use nsh_core::prelude::*;

use crate::{preferences::Language, store::Store};

pub(crate) fn tr(message: &str) -> SharedString {
    let language = use_context::<Store>()
        .map(|store| store.read(|state| state.preferences.language))
        .unwrap_or_default();
    translate(language, message).to_owned().into()
}

pub(crate) fn translate(language: Language, message: &str) -> &str {
    if language == Language::Chinese {
        return message;
    }
    match message {
        "正在打开本地资料库…" => "Opening local library…",
        "重试打开" => "Try opening again",
        "重试" => "Retry",
        "正在载入…" => "Loading…",
        "{} 篇文章 · 本地存储" => "{} articles · Stored locally",
        "J / K 浏览 · M 已读 · S 收藏" => "J / K Navigate · M Read · S Star",
        "添加订阅" => "Add subscription",
        "订阅" => "Subscriptions",
        "尚无订阅" => "No subscriptions yet",
        "服务器与同步" => "Settings & sync",
        "文章列表操作" => "Article list actions",
        "重新载入文章" => "Reload articles",
        "抓取当前订阅" => "Fetch current subscription",
        "取消本地订阅…" => "Unsubscribe locally…",
        "取消本地订阅？" => "Unsubscribe on this device?",
        "只删除此设备上的订阅，不删除远端；已导入的来源不会在下次拉取时自动添加回来。" => {
            "Remove only from this device, not the server. Imported sources will not be added again on the next import."
        }
        "保留" => "Keep",
        "取消订阅" => "Unsubscribe",
        "搜索文章" => "Search articles",
        "这里还没有文章" => "No articles yet",
        "添加订阅，或调整筛选条件。" => "Add a subscription or adjust your filters.",
        "返回" => "Back",
        "标为未读" => "Mark unread",
        "标为已读" => "Mark read",
        "取消收藏" => "Unstar",
        "收藏" => "Starred",
        "收藏文章" => "Star",
        "打开原文" => "Open original",
        "正在载入文章…" => "Loading article…",
        "请稍候。" => "Please wait.",
        "留一点时间，读点好内容" => "Make time for a good read",
        "从列表中选择一篇文章开始阅读。" => {
            "Choose an article from the list to start reading."
        }
        "[图片：请打开原文查看]" => "[Image: open the original to view]",
        "[图片：{}]" => "[Image: {}]",
        "Feed 地址" => "Feed URL",
        "默认只保存到本地。添加后可在服务器与同步中选择同步目标。" => {
            "Saved locally by default. Choose a sync destination in Settings & sync after adding."
        }
        "取消" => "Cancel",
        "添加" => "Add",
        "完成" => "Done",
        "本地资料库" => "Local library",
        "无需服务器 · 数据保存在此设备" => "No server needed · Stored on this device",
        "已连接 · 兼容模式" => "Connected · Compatibility mode",
        "需要登录" => "Sign in required",
        "添加服务器…" => "Add server…",
        "连接远端 Raindrop" => "Connect to Raindrop",
        "登录后导入订阅与可见 AI 配置，不上传本地存量，不复制 API 密钥。" => {
            "Import subscriptions and visible AI settings after sign-in. Existing local data stays local; API keys are never copied."
        }
        "HTTPS 服务地址" => "HTTPS server URL",
        "用户名或邮箱" => "Username or email",
        "密码" => "Password",
        "连接并导入" => "Connect & import",
        "可选的远端连接" => "Optional server connections",
        "本地阅读不依赖远端。添加服务器后，可导入订阅和 AI 配置，或明确推送选中的本地订阅。" => {
            "Reading works without a server. Connect one to import subscriptions and AI settings, or explicitly push selected local subscriptions."
        }
        "正在处理，请稍候…" => "Working, please wait…",
        "外观与语言" => "Appearance & language",
        "仅影响此设备，自动保存。" => {
            "Applies to this device only. Saved automatically."
        }
        "语言" => "Language",
        "简体中文" => "简体中文",
        "主题" => "Theme",
        "跟随系统" => "System",
        "浅色" => "Light",
        "深色" => "Dark",
        "无法读取外观设置，已使用默认值；原有设置未被覆盖。" => {
            "Could not read appearance settings. Using defaults without overwriting the saved settings."
        }
        "无法保存外观设置，请检查磁盘空间后重试。" => {
            "Could not save appearance settings. Check disk space and try again."
        }
        "拉取订阅与 AI 配置" => "Import subscriptions & AI settings",
        "重新登录" => "Sign in again",
        "移除连接…" => "Remove connection…",
        "移除服务器连接？" => "Remove server connection?",
        "保留已导入本地的订阅；取消此连接尚未完成的推送任务。" => {
            "Keep imported local subscriptions and cancel unfinished push tasks for this connection."
        }
        "移除连接" => "Remove connection",
        "同步范围" => "Sync scope",
        "兼容导入：拉取新增订阅，关联相同 Feed；保留本地标题、分类和顺序。远端修改不会直接覆盖本地。" => {
            "Compatibility import: add new subscriptions and link matching feeds. Keep local titles, categories and order; remote edits never overwrite them."
        }
        "单向推送：只推送下方明确选中的新增订阅，不修改或删除远端已有订阅。" => {
            "One-way push: add only the subscriptions selected below. Never edit or delete existing server subscriptions."
        }
        "可靠双向：当前服务器未协商修订、幂等和删除记录协议，因此暂不可用。" => {
            "Reliable two-way sync is unavailable until the server supports revision, idempotency and deletion protocols."
        }
        "远端 AI 配置" => "Remote AI settings",
        "以下仅为远端引用，不含 API 密钥。尚未关联远端文章身份，不会自动执行 AI，也不会静默改用本地 Provider。" => {
            "References only; no API keys. Remote article identities are not linked, so AI is not run automatically and never silently falls back to a local provider."
        }
        "插件：{} · AI：{}" => "Plugin: {} · AI: {}",
        "已启用" => "Enabled",
        "未启用" => "Disabled",
        "远端已启用" => "Enabled on server",
        "远端未启用" => "Disabled on server",
        "推送新增订阅" => "Push new subscriptions",
        "可离线保存发送意图。连接后点「发送待处理」；结果未知时只核对，不自动重发。" => {
            "Queue tasks offline, then send when connected. Unknown outcomes are checked, never automatically resent."
        }
        "发送待处理 / 核对结果" => "Send pending / Check outcomes",
        "加入待发送" => "Queue for sending",
        "取消此任务" => "Cancel task",
        "同步记录无法读取，已禁用远端操作；本地阅读仍可使用。请保留数据库以便恢复。" => {
            "Sync records could not be read. Remote actions are disabled; local reading still works. Keep the database for recovery."
        }
        "已添加订阅，正在后台获取文章。" => {
            "Subscription added. Fetching articles in the background."
        }
        "已安排抓取；文章就绪后重新载入列表。" => {
            "Fetch scheduled. Reload the list when articles are ready."
        }
        "已导入 {} 个订阅，关联 {} 个；{} 个远端修改保留待核对，{} 个已取消的来源未重新添加。" => {
            "Imported {} subscriptions, linked {}; {} remote edits need review, {} removed sources were not re-added."
        }
        "已处理待发送任务；结果未知的任务不会自动重发。" => {
            "Pending tasks processed. Tasks with unknown outcomes will not be automatically resent."
        }
        "请输入 HTTPS 服务地址；本机调试可使用 http://127.0.0.1:端口。" => {
            "Enter an HTTPS server URL. Local testing may use http://127.0.0.1:port."
        }
        "无法连接服务器。请检查网络和服务地址后重试。" => {
            "Cannot connect. Check your network and server URL, then try again."
        }
        "登录信息无效或会话已过期，请重新登录。" => {
            "Invalid credentials or expired session. Sign in again."
        }
        "服务器拒绝了操作，请重新登录后重试。" => {
            "The server refused this action. Sign in again and retry."
        }
        "请求过于频繁，请稍后重试。" => "Too many requests. Try again later.",
        "服务器暂时无法处理请求，请稍后重试。" => {
            "The server is temporarily unavailable. Try again later."
        }
        "服务器响应不兼容，请确认使用 Raindrop 0.7 或更新版本。" => {
            "Incompatible server response. Use Raindrop 0.7 or newer."
        }
        "无法打开本地数据，请检查应用数据目录的权限与可用空间。" => {
            "Cannot open local data. Check data directory permissions and free disk space."
        }
        "本地数据正在由另一个 Raindrop Desktop 实例使用。" => {
            "Another Raindrop Desktop instance is using this local library."
        }
        "订阅已在另一端修改，请检查同步冲突后重试。" => {
            "The subscription changed on the other device. Review the conflict before retrying."
        }
        "无标题文章" => "Untitled article",
        "全部文章" => "All articles",
        "未读" => "Unread",
        "等待推送" => "Queued",
        "结果未知 · 需核对" => "Unknown outcome · Needs checking",
        "已推送" => "Confirmed",
        "已取消" => "Canceled",
        _ => message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_desktop_interface_messages_have_english_translations() {
        for source in [
            include_str!("lib.rs"),
            include_str!("reader.rs"),
            include_str!("settings.rs"),
            include_str!("store.rs"),
            include_str!("api.rs"),
            include_str!("sync.rs"),
        ] {
            for message in source.split('"').skip(1).step_by(2) {
                if message
                    .chars()
                    .any(|character| ('\u{3400}'..='\u{9fff}').contains(&character))
                    && message != "简体中文"
                {
                    assert_ne!(
                        translate(Language::English, message),
                        message,
                        "untranslated message: {message}"
                    );
                }
            }
        }
        assert_eq!(translate(Language::Chinese, "未读"), "未读");
        assert_eq!(
            translate(Language::English, "Unrecognized article title"),
            "Unrecognized article title"
        );
    }
}
