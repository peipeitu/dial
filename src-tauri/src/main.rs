#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod scan_cache;

use std::{
    collections::HashMap,
    env,
    fs::{self, File},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose, Engine};
use chrono::{DateTime, Datelike, Duration, Local, TimeZone, Utc};
use rusqlite::{Connection, OpenFlags};
use scan_cache::{cache_source_key, ScanCacheStore, ScanDiagnostics, ScanRequest, ScannedFile};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};
#[cfg(any(test, target_os = "windows"))]
use tauri::{PhysicalPosition, PhysicalSize};
#[cfg(any(windows, target_os = "linux"))]
use tauri_plugin_updater::UpdaterExt;
use walkdir::WalkDir;
#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::{LPARAM, WPARAM},
    UI::WindowsAndMessaging::{CreateIcon, DestroyIcon, SendMessageW, HICON, ICON_BIG, WM_SETICON},
};

#[cfg(any(windows, target_os = "linux"))]
const UPDATE_ENDPOINT: &str =
    "https://github.com/peipeitu/dial/releases/latest/download/latest.json";
const MIN_CHART_DAYS: u32 = 7;
const DEFAULT_CHART_DAYS: u32 = 30;
const MAX_CHART_DAYS: u32 = 90;
const DEFAULT_AUTO_REFRESH_ENABLED: bool = false;
const DEFAULT_AUTO_REFRESH_MINUTES: u32 = 30;
const MAX_AUTO_REFRESH_MINUTES: u32 = 1440;
const CODEX_USD_PER_MILLION_TOKENS: f64 = 1.0;
const DEFAULT_SCAN_MAX_DEPTH: usize = 8;
const DEFAULT_SCAN_MAX_FILES: usize = 20_000;
const CHATGPT_SQLITE_ROW_SCAN_LIMIT: usize = 2_000;
const CLAUDE_ESTIMATED_5H_TOKEN_LIMIT: u64 = 500_000;
const CLAUDE_ESTIMATED_WEEKLY_TOKEN_LIMIT: u64 = 2_500_000;
const TRAY_ID: &str = "ai-usage-status";
const TRAY_STATUS_WINDOW_LABEL: &str = "tray-status";
const TRAY_OPEN_MENU_ID: &str = "tray-open";
const TRAY_QUIT_MENU_ID: &str = "tray-quit";
static SETTINGS_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrayLanguage {
    Zh,
    En,
}

#[derive(Clone, Debug)]
struct TrayDisplayState {
    language: TrayLanguage,
    active_provider: String,
    remaining_percent: Option<u8>,
    theme: String,
    accent_color: String,
    auto_refresh_enabled: bool,
    auto_refresh_minutes: u32,
    is_refreshing: bool,
    refresh_failed: bool,
    refresh_in_flight: bool,
    pending_refresh_request_id: Option<u64>,
    latest_refresh_request_id: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrayStatusPayload {
    provider: String,
    provider_label: String,
    remaining_percent: Option<u8>,
    usage_label: String,
    status_label: String,
    open_label: String,
    language: String,
    theme: String,
    accent_color: String,
}

struct TrayUiState {
    display: Mutex<TrayDisplayState>,
    auto_refresh_notify: Arc<tokio::sync::Notify>,
    stats_scan_lock: Arc<Mutex<()>>,
    scan_cache: Arc<ScanCacheStore>,
    scan_diagnostics: Arc<Mutex<HashMap<String, ScanDiagnostics>>>,
    open_item: MenuItem<tauri::Wry>,
    quit_item: MenuItem<tauri::Wry>,
}

#[cfg(target_os = "windows")]
struct WindowsTaskbarIcon(Mutex<isize>);

#[cfg(target_os = "windows")]
impl WindowsTaskbarIcon {
    fn replace(&self, icon: isize) {
        let mut current = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = std::mem::replace(&mut *current, icon);
        if previous != 0 {
            let _ = unsafe { DestroyIcon(HICON(previous as _)) };
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for WindowsTaskbarIcon {
    fn drop(&mut self) {
        let icon = self
            .0
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *icon != 0 {
            let _ = unsafe { DestroyIcon(HICON(*icon as _)) };
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    active_provider: String,
    #[serde(default = "default_enabled_providers")]
    enabled_providers: Vec<String>,
    #[serde(default)]
    codex_home: String,
    #[serde(default)]
    claude_home: String,
    #[serde(default)]
    copilot_home: String,
    #[serde(default)]
    cursor_home: String,
    #[serde(default)]
    chatgpt_home: String,
    #[serde(default = "default_language")]
    language: String,
    theme: String,
    accent_color: String,
    chart_days: u32,
    #[serde(default = "default_auto_refresh_enabled")]
    auto_refresh_enabled: bool,
    #[serde(default = "default_auto_refresh_minutes")]
    auto_refresh_minutes: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Account {
    display_name: String,
    initials: String,
    plan_type: Option<String>,
    plan_label: String,
    plan_monthly_usd: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChartSettings {
    chart_days: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateInfo {
    supported: bool,
    available: bool,
    current_version: String,
    version: Option<String>,
    notes: Option<String>,
    published_at: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Pricing {
    label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    checked_at: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Featured {
    today_tokens: u64,
    today_cost: f64,
    period_cost: f64,
    period_tokens: u64,
    latest_token_usage: u64,
    cost_available: bool,
    cost_estimated_from_token_events: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Totals {
    threads: usize,
    active_threads: usize,
    archived_threads: usize,
    total_tokens: u64,
    updated_this_week: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DailyPoint {
    date: String,
    label: String,
    threads: u64,
    tokens: u64,
    cost: f64,
}

#[derive(Clone, Debug, Serialize)]
struct RankItem {
    name: String,
    value: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LatestThread {
    id: String,
    title: String,
    model: String,
    source: String,
    tokens_used: u64,
    updated_at: Option<String>,
    cwd: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RateLimitWindow {
    id: String,
    label: String,
    used_percent: f64,
    remaining_percent: f64,
    window_minutes: u64,
    resets_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RateLimits {
    updated_at: Option<String>,
    plan_type: Option<String>,
    reached_type: Option<String>,
    windows: Vec<RateLimitWindow>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RateLimitHistoryPoint {
    timestamp: String,
    used_percent: f64,
    remaining_percent: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RateLimitHistorySeries {
    id: String,
    label: String,
    window_minutes: u64,
    points: Vec<RateLimitHistoryPoint>,
}

type RawRateLimitHistoryPoint = (i64, f64, f64);
type RateLimitHistoryGroups = HashMap<(String, u64), (String, Vec<RawRateLimitHistoryPoint>)>;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivityWindow {
    id: String,
    label: String,
    count: u64,
    window_minutes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivitySummary {
    updated_at: Option<String>,
    windows: Vec<ActivityWindow>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Stats {
    generated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    account: Account,
    settings: ChartSettings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pricing: Option<Pricing>,
    featured: Featured,
    rate_limits: Option<RateLimits>,
    rate_limit_history: Vec<RateLimitHistorySeries>,
    activity: Option<ActivitySummary>,
    totals: Totals,
    daily_series: Vec<DailyPoint>,
    models: Vec<RankItem>,
    sources: Vec<RankItem>,
    workspaces: Vec<RankItem>,
    latest_threads: Vec<LatestThread>,
    paths: Value,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageHistory {
    #[serde(default)]
    codex: HashMap<String, DailyTokenHistory>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DailyTokenHistory {
    #[serde(default)]
    daily_tokens: HashMap<String, u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Thread {
    id: String,
    title: String,
    source: String,
    model: String,
    cwd: String,
    archived: bool,
    tokens_used: u64,
    created_at_ms: i64,
    updated_at_ms: i64,
    rollout_path: String,
    usage_events: Vec<UsageEvent>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct UsageEvent {
    thread_id: String,
    timestamp_ms: i64,
    model: String,
    total_tokens: u64,
    plan_type: Option<String>,
    rate_limits: Option<RateLimits>,
}

struct StatsScanResult {
    stats: Stats,
    diagnostics: ScanDiagnostics,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChooseHomeResult {
    settings: Settings,
    stats: Stats,
}

fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn default_settings() -> Settings {
    Settings {
        active_provider: "codex".to_string(),
        enabled_providers: default_enabled_providers(),
        codex_home: String::new(),
        claude_home: String::new(),
        copilot_home: String::new(),
        cursor_home: String::new(),
        chatgpt_home: String::new(),
        language: default_language(),
        theme: "system".to_string(),
        accent_color: "blue".to_string(),
        chart_days: DEFAULT_CHART_DAYS,
        auto_refresh_enabled: DEFAULT_AUTO_REFRESH_ENABLED,
        auto_refresh_minutes: DEFAULT_AUTO_REFRESH_MINUTES,
    }
}

fn default_language() -> String {
    "auto".to_string()
}

fn default_auto_refresh_minutes() -> u32 {
    DEFAULT_AUTO_REFRESH_MINUTES
}

fn default_auto_refresh_enabled() -> bool {
    DEFAULT_AUTO_REFRESH_ENABLED
}

fn default_enabled_providers() -> Vec<String> {
    ["codex", "claude", "copilot", "cursor", "chatgpt"]
        .iter()
        .map(|provider| provider.to_string())
        .collect()
}

fn normalize_settings(settings: Settings) -> Settings {
    let providers = ["codex", "claude", "copilot", "cursor", "chatgpt"];
    let languages = ["auto", "zh", "en"];
    let themes = ["system", "light", "dark"];
    let accents = [
        "blue",
        "turquoise",
        "green",
        "purple",
        "red",
        "orange",
        "graphite",
    ];

    let mut enabled_providers = Vec::new();
    for provider in settings.enabled_providers {
        if providers.contains(&provider.as_str()) && !enabled_providers.contains(&provider) {
            enabled_providers.push(provider);
        }
    }
    if enabled_providers.is_empty() {
        enabled_providers = default_enabled_providers();
    }

    let active_provider = if providers.contains(&settings.active_provider.as_str())
        && enabled_providers.contains(&settings.active_provider)
    {
        settings.active_provider
    } else {
        enabled_providers
            .first()
            .cloned()
            .unwrap_or_else(|| "codex".to_string())
    };

    Settings {
        active_provider,
        enabled_providers,
        codex_home: settings.codex_home,
        claude_home: settings.claude_home,
        copilot_home: settings.copilot_home,
        cursor_home: settings.cursor_home,
        chatgpt_home: settings.chatgpt_home,
        language: if languages.contains(&settings.language.as_str()) {
            settings.language
        } else {
            default_language()
        },
        theme: if themes.contains(&settings.theme.as_str()) {
            settings.theme
        } else {
            "system".to_string()
        },
        accent_color: if accents.contains(&settings.accent_color.as_str()) {
            settings.accent_color
        } else {
            "blue".to_string()
        },
        chart_days: settings.chart_days.clamp(MIN_CHART_DAYS, MAX_CHART_DAYS),
        auto_refresh_enabled: settings.auto_refresh_enabled,
        auto_refresh_minutes: settings
            .auto_refresh_minutes
            .clamp(1, MAX_AUTO_REFRESH_MINUTES),
    }
}

fn settings_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| home_dir().join(".config"))
        .join("ai-usage")
        .join("settings.json")
}

fn usage_history_path() -> PathBuf {
    settings_path().with_file_name("usage-history.json")
}

fn scan_cache_dir() -> PathBuf {
    settings_path().with_file_name("scan-cache")
}

fn scan_request(
    provider: &str,
    parser_version: &str,
    roots: &[PathBuf],
    files: Vec<PathBuf>,
    force: bool,
) -> ScanRequest {
    ScanRequest {
        provider: provider.to_string(),
        parser_version: parser_version.to_string(),
        source_key: cache_source_key(roots),
        cache_path: scan_cache_dir().join(format!("{provider}.json")),
        files,
        force,
    }
}

fn finish_scan(
    provider: &str,
    started_at: Instant,
    stats: Stats,
    mut diagnostics: ScanDiagnostics,
) -> StatsScanResult {
    diagnostics.provider = provider.to_string();
    diagnostics.elapsed_ms = started_at
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX);
    diagnostics.record();
    StatsScanResult { stats, diagnostics }
}

fn remember_scan_diagnostics(
    diagnostics_store: &Mutex<HashMap<String, ScanDiagnostics>>,
    diagnostics: &ScanDiagnostics,
) {
    diagnostics_store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(diagnostics.provider.clone(), diagnostics.clone());
}

fn load_settings() -> Settings {
    let path = settings_path();
    let Ok(content) = fs::read_to_string(path) else {
        return default_settings();
    };

    match serde_json::from_str::<Settings>(&content) {
        Ok(settings) => normalize_settings(settings),
        Err(_) => {
            quarantine_invalid_settings(&settings_path());
            default_settings()
        }
    }
}

fn quarantine_invalid_settings(path: &Path) {
    if !path.exists() {
        return;
    }
    let backup_name = format!(
        "settings.invalid-{}.json",
        Utc::now().format("%Y%m%d%H%M%S%3f")
    );
    let backup_path = path.with_file_name(backup_name);
    let _ = fs::rename(path, backup_path);
}

fn unique_settings_sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("settings.json");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let counter = SETTINGS_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);

    path.with_file_name(format!(
        ".{file_name}.{}.{}.{}.{}",
        std::process::id(),
        nonce,
        counter,
        suffix
    ))
}

fn unique_settings_temp_path(path: &Path) -> PathBuf {
    unique_settings_sibling_path(path, "tmp")
}

#[cfg(target_os = "windows")]
fn replace_settings_file(temp_path: &Path, path: &Path) -> Result<(), String> {
    if !path.exists() {
        return fs::rename(temp_path, path).map_err(|error| error.to_string());
    }

    let backup_path = unique_settings_sibling_path(path, "bak");
    fs::rename(path, &backup_path).map_err(|error| error.to_string())?;

    match fs::rename(temp_path, path) {
        Ok(()) => {
            let _ = fs::remove_file(backup_path);
            Ok(())
        }
        Err(error) => {
            let restore_result = fs::rename(&backup_path, path);
            if let Err(restore_error) = restore_result {
                Err(format!(
                    "Unable to replace settings file: {error}; also failed to restore previous settings: {restore_error}"
                ))
            } else {
                Err(error.to_string())
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn replace_settings_file(temp_path: &Path, path: &Path) -> Result<(), String> {
    fs::rename(temp_path, path).map_err(|error| error.to_string())
}

fn save_settings(settings: &Settings) -> Result<(), String> {
    let path = settings_path();
    write_json_file(&path, settings)
}

fn write_json_file<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let content = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    let temp_path = unique_settings_temp_path(path);
    let write_result = (|| -> Result<(), String> {
        let mut file = File::create(&temp_path).map_err(|error| error.to_string())?;
        file.write_all(format!("{content}\n").as_bytes())
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        drop(file);

        replace_settings_file(&temp_path, path)
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    write_result
}

fn load_usage_history() -> UsageHistory {
    load_usage_history_from_path(&usage_history_path())
}

fn load_usage_history_from_path(path: &Path) -> UsageHistory {
    let Ok(content) = fs::read_to_string(path) else {
        return UsageHistory::default();
    };

    serde_json::from_str::<UsageHistory>(&content).unwrap_or_default()
}

fn save_usage_history(history: &UsageHistory) -> Result<(), String> {
    write_json_file(&usage_history_path(), history)
}

fn current_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[cfg(any(windows, target_os = "linux"))]
fn updater_public_key() -> Option<&'static str> {
    #[cfg(debug_assertions)]
    {
        None
    }

    #[cfg(not(debug_assertions))]
    {
        option_env!("AI_USAGE_UPDATER_PUBLIC_KEY").and_then(|key| {
            let key = key.trim();
            if key.is_empty() {
                None
            } else {
                Some(key)
            }
        })
    }
}

fn unsupported_update_info() -> UpdateInfo {
    UpdateInfo {
        supported: false,
        available: false,
        current_version: current_app_version(),
        version: None,
        notes: None,
        published_at: None,
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn updater_published_at(date: time::OffsetDateTime) -> Option<String> {
    DateTime::<Utc>::from_timestamp(date.unix_timestamp(), date.nanosecond())
        .map(|date| date.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn provider_label(provider: &str) -> &'static str {
    match provider {
        "claude" => "Claude Code",
        "copilot" => "GitHub Copilot",
        "cursor" => "Cursor",
        "chatgpt" => "ChatGPT",
        _ => "Codex",
    }
}

fn tray_language(language: &str) -> TrayLanguage {
    let language = if language == "auto" {
        env::var("LANG").unwrap_or_default()
    } else {
        language.to_string()
    };
    if language.to_lowercase().starts_with("zh") {
        TrayLanguage::Zh
    } else {
        TrayLanguage::En
    }
}

fn tray_menu_labels(language: TrayLanguage) -> (&'static str, &'static str) {
    match language {
        TrayLanguage::Zh => ("打开 Dial", "退出"),
        TrayLanguage::En => ("Open Dial", "Quit"),
    }
}

impl TrayDisplayState {
    fn update_usage(&mut self, provider: &str, remaining_percent: Option<u8>) -> bool {
        if self.active_provider != provider {
            return false;
        }
        self.remaining_percent = remaining_percent;
        self.refresh_failed = false;
        true
    }

    fn next_refresh_request(&mut self) -> u64 {
        self.latest_refresh_request_id = self.latest_refresh_request_id.wrapping_add(1).max(1);
        self.is_refreshing = true;
        self.refresh_failed = false;
        self.latest_refresh_request_id
    }
}

fn tray_tooltip(display: &TrayDisplayState) -> String {
    let provider = provider_label(&display.active_provider);
    match (display.language, display.remaining_percent) {
        (TrayLanguage::Zh, Some(percent)) => {
            format!("Dial · {provider} · 剩余 {percent}%")
        }
        (TrayLanguage::En, Some(percent)) => {
            format!("Dial · {provider} · {percent}% remaining")
        }
        (_, None) => format!("Dial · {provider}"),
    }
}

fn tray_remaining_percent(rate_limits: Option<&RateLimits>) -> Option<u8> {
    let windows = &rate_limits?.windows;
    let window = windows
        .iter()
        .filter(|window| is_available_rate_limit_window(window))
        .find(|window| window.id == "primary")
        .or_else(|| {
            windows
                .iter()
                .find(|window| is_available_rate_limit_window(window))
        })?;
    Some(window.remaining_percent.clamp(0.0, 100.0).round() as u8)
}

fn is_available_rate_limit_window(window: &RateLimitWindow) -> bool {
    window.window_minutes > 0
        && window.used_percent.is_finite()
        && window.remaining_percent.is_finite()
}

fn tray_status_payload(display: &TrayDisplayState) -> TrayStatusPayload {
    let (usage_label, open_label, language) = match display.language {
        TrayLanguage::Zh => ("剩余用量", "打开详情", "zh-CN"),
        TrayLanguage::En => ("Remaining usage", "Open details", "en"),
    };
    let status_label = match display.language {
        TrayLanguage::Zh if display.is_refreshing => "正在更新...".to_string(),
        TrayLanguage::En if display.is_refreshing => "Updating...".to_string(),
        TrayLanguage::Zh if display.refresh_failed && display.remaining_percent.is_some() => {
            "更新失败，显示上次数据".to_string()
        }
        TrayLanguage::En if display.refresh_failed && display.remaining_percent.is_some() => {
            "Update failed, showing previous data".to_string()
        }
        TrayLanguage::Zh if display.refresh_failed => "更新失败".to_string(),
        TrayLanguage::En if display.refresh_failed => "Update failed".to_string(),
        TrayLanguage::Zh if display.remaining_percent.is_none() => "暂无可用数据".to_string(),
        TrayLanguage::En if display.remaining_percent.is_none() => "Usage unavailable".to_string(),
        TrayLanguage::Zh if display.auto_refresh_enabled => {
            format!("每 {} 分钟更新", display.auto_refresh_minutes)
        }
        TrayLanguage::En if display.auto_refresh_enabled => {
            format!("Updates every {} min", display.auto_refresh_minutes)
        }
        TrayLanguage::Zh => "自动更新已关闭".to_string(),
        TrayLanguage::En => "Auto refresh is off".to_string(),
    };

    TrayStatusPayload {
        provider: display.active_provider.clone(),
        provider_label: provider_label(&display.active_provider).to_string(),
        remaining_percent: display.remaining_percent,
        usage_label: usage_label.to_string(),
        status_label,
        open_label: open_label.to_string(),
        language: language.to_string(),
        theme: display.theme.clone(),
        accent_color: display.accent_color.clone(),
    }
}

#[cfg(any(test, target_os = "windows"))]
fn tray_status_window_position(
    tray_position: PhysicalPosition<f64>,
    tray_size: PhysicalSize<u32>,
    window_size: PhysicalSize<u32>,
    monitor_position: PhysicalPosition<i32>,
    monitor_size: PhysicalSize<u32>,
) -> PhysicalPosition<i32> {
    const MARGIN: f64 = 8.0;

    let monitor_left = f64::from(monitor_position.x);
    let monitor_top = f64::from(monitor_position.y);
    let monitor_right = monitor_left + f64::from(monitor_size.width);
    let monitor_bottom = monitor_top + f64::from(monitor_size.height);
    let tray_center_x = tray_position.x + f64::from(tray_size.width) / 2.0;
    let tray_center_y = tray_position.y + f64::from(tray_size.height) / 2.0;
    let window_width = f64::from(window_size.width);
    let window_height = f64::from(window_size.height);
    let distances = [
        tray_center_y - monitor_top,
        monitor_right - tray_center_x,
        monitor_bottom - tray_center_y,
        tray_center_x - monitor_left,
    ];
    let nearest_edge = distances
        .iter()
        .enumerate()
        .min_by(|left, right| left.1.total_cmp(right.1))
        .map(|(index, _)| index)
        .unwrap_or(2);

    let (x, y) = match nearest_edge {
        0 => (
            tray_center_x - window_width / 2.0,
            tray_position.y + f64::from(tray_size.height) + MARGIN,
        ),
        1 => (
            tray_position.x - window_width - MARGIN,
            tray_center_y - window_height / 2.0,
        ),
        3 => (
            tray_position.x + f64::from(tray_size.width) + MARGIN,
            tray_center_y - window_height / 2.0,
        ),
        _ => (
            tray_center_x - window_width / 2.0,
            tray_position.y - window_height - MARGIN,
        ),
    };
    let min_x = monitor_left + MARGIN;
    let min_y = monitor_top + MARGIN;
    let max_x = (monitor_right - window_width - MARGIN).max(min_x);
    let max_y = (monitor_bottom - window_height - MARGIN).max(min_y);

    PhysicalPosition::new(
        x.clamp(min_x, max_x).round() as i32,
        y.clamp(min_y, max_y).round() as i32,
    )
}

fn emit_tray_status(app: &tauri::AppHandle, display: &TrayDisplayState) {
    let _ = app.emit_to(
        TRAY_STATUS_WINDOW_LABEL,
        "tray-status-updated",
        tray_status_payload(display),
    );
}

fn apply_tray_display(app: &tauri::AppHandle, display: &TrayDisplayState) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };

    #[cfg(not(target_os = "windows"))]
    {
        let title = display
            .remaining_percent
            .map(|percent| format!("{percent}%"))
            .unwrap_or_else(|| "--%".to_string());
        let _ = tray.set_title(Some(title));
    }
    let _ = tray.set_tooltip(Some(tray_tooltip(display)));
    emit_tray_status(app, display);
}

fn sync_tray_settings(app: &tauri::AppHandle, settings: &Settings) {
    let Some(state) = app.try_state::<TrayUiState>() else {
        return;
    };
    let (display, auto_refresh_changed) = {
        let mut display = state
            .display
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let provider_changed = display.active_provider != settings.active_provider;
        let auto_refresh_changed = display.auto_refresh_enabled != settings.auto_refresh_enabled
            || display.auto_refresh_minutes != settings.auto_refresh_minutes;
        if provider_changed {
            display.active_provider = settings.active_provider.clone();
            display.remaining_percent = None;
            display.refresh_failed = false;
            display.latest_refresh_request_id =
                display.latest_refresh_request_id.wrapping_add(1).max(1);
            display.is_refreshing = false;
            display.pending_refresh_request_id = None;
        }
        display.theme = settings.theme.clone();
        display.accent_color = settings.accent_color.clone();
        display.auto_refresh_enabled = settings.auto_refresh_enabled;
        display.auto_refresh_minutes = settings.auto_refresh_minutes;
        (display.clone(), auto_refresh_changed)
    };
    apply_tray_display(app, &display);
    if auto_refresh_changed {
        state.auto_refresh_notify.notify_one();
    }
}

fn begin_external_tray_refresh(app: &tauri::AppHandle, provider: &str) -> Option<u64> {
    let state = app.try_state::<TrayUiState>()?;
    let display = {
        let mut display = state
            .display
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if display.active_provider != provider {
            return None;
        }
        let request_id = display.next_refresh_request();
        display.pending_refresh_request_id = None;
        (request_id, display.clone())
    };
    apply_tray_display(app, &display.1);
    Some(display.0)
}

fn complete_external_tray_refresh(
    app: &tauri::AppHandle,
    request_id: u64,
    provider: &str,
    result: Result<&Stats, &str>,
) {
    let Some(state) = app.try_state::<TrayUiState>() else {
        return;
    };
    let display = {
        let mut display = state
            .display
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if display.latest_refresh_request_id != request_id || display.active_provider != provider {
            return;
        }
        display.is_refreshing = false;
        match result {
            Ok(stats) => {
                display.update_usage(provider, tray_remaining_percent(stats.rate_limits.as_ref()));
            }
            Err(_) => display.refresh_failed = true,
        }
        display.clone()
    };
    apply_tray_display(app, &display);
}

fn request_tray_refresh(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<TrayUiState>() else {
        return;
    };
    let (request_id, should_start, display) = {
        let mut display = state
            .display
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let request_id = display.next_refresh_request();
        let should_start = if display.refresh_in_flight {
            display.pending_refresh_request_id = Some(request_id);
            false
        } else {
            display.refresh_in_flight = true;
            true
        };
        (request_id, should_start, display.clone())
    };
    apply_tray_display(app, &display);
    if should_start {
        tauri::async_runtime::spawn(refresh_active_tray_usage(app.clone(), request_id));
    }
}

async fn refresh_active_tray_usage(app: tauri::AppHandle, mut request_id: u64) {
    loop {
        let Some(state) = app.try_state::<TrayUiState>() else {
            return;
        };
        let scan_lock = state.stats_scan_lock.clone();
        let scan_cache = state.scan_cache.clone();
        let scan_diagnostics = state.scan_diagnostics.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            let _scan = scan_lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let settings = load_settings();
            let provider = settings.active_provider.clone();
            let result = read_stats_for_provider(&settings, &provider, &scan_cache, false)?;
            remember_scan_diagnostics(&scan_diagnostics, &result.diagnostics);
            Ok::<_, String>((provider, result.stats))
        })
        .await
        .map_err(|error| error.to_string())
        .and_then(|result| result);

        let Some(state) = app.try_state::<TrayUiState>() else {
            return;
        };
        let (next_request_id, updated_display) = {
            let mut display = state
                .display
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let is_latest = display.latest_refresh_request_id == request_id;
            let mut updated_display = None;

            if let Some(pending_request_id) = display.pending_refresh_request_id.take() {
                (Some(pending_request_id), updated_display)
            } else {
                display.refresh_in_flight = false;
                if is_latest {
                    display.is_refreshing = false;
                    match &result {
                        Ok((provider, stats)) if display.active_provider == *provider => {
                            display.update_usage(
                                provider,
                                tray_remaining_percent(stats.rate_limits.as_ref()),
                            );
                        }
                        _ => display.refresh_failed = true,
                    }
                    updated_display = Some(display.clone());
                }
                (None, updated_display)
            }
        };

        if let Some(display) = updated_display {
            apply_tray_display(&app, &display);
        }
        if let Some(next_request_id) = next_request_id {
            request_id = next_request_id;
        } else {
            break;
        }
    }
}

fn tray_auto_refresh_delay(settings: &Settings) -> Option<std::time::Duration> {
    if !settings.auto_refresh_enabled {
        return None;
    }
    Some(std::time::Duration::from_secs(
        u64::from(settings.auto_refresh_minutes) * 60,
    ))
}

fn start_tray_auto_refresh(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let Some(state) = app.try_state::<TrayUiState>() else {
                return;
            };
            let notify = state.auto_refresh_notify.clone();
            match tray_auto_refresh_delay(&load_settings()) {
                Some(delay) => {
                    if tokio::time::timeout(delay, notify.notified())
                        .await
                        .is_err()
                    {
                        request_tray_refresh(&app);
                    }
                }
                None => notify.notified().await,
            }
        }
    });
}

fn show_main_window(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

#[tauri::command]
fn get_tray_status(app: tauri::AppHandle) -> Result<TrayStatusPayload, String> {
    let state = app
        .try_state::<TrayUiState>()
        .ok_or_else(|| "tray state is not initialized".to_string())?;
    let display = state
        .display
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(tray_status_payload(&display))
}

#[tauri::command]
fn open_main_window(app: tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(TRAY_STATUS_WINDOW_LABEL) {
        let _ = window.hide();
    }
    show_main_window(&app);
}

#[cfg(target_os = "windows")]
fn toggle_tray_status_window(
    app: &tauri::AppHandle,
    tray_position: PhysicalPosition<f64>,
    tray_size: PhysicalSize<u32>,
) {
    let Some(window) = app.get_webview_window(TRAY_STATUS_WINDOW_LABEL) else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }

    let tray_center_x = tray_position.x + f64::from(tray_size.width) / 2.0;
    let tray_center_y = tray_position.y + f64::from(tray_size.height) / 2.0;
    if let Ok(Some(monitor)) = app.monitor_from_point(tray_center_x, tray_center_y) {
        let window_size = window
            .outer_size()
            .unwrap_or_else(|_| PhysicalSize::new(320, 156));
        let position = tray_status_window_position(
            tray_position,
            tray_size,
            window_size,
            *monitor.position(),
            *monitor.size(),
        );
        let _ = window.set_position(position);
    }

    if let Some(state) = app.try_state::<TrayUiState>() {
        let display = state
            .display
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        emit_tray_status(app, &display);
    }
    let _ = window.show();
    let _ = window.set_focus();
    request_tray_refresh(app);
}

#[cfg(any(test, target_os = "windows"))]
fn windows_icon_size(base_size: u32, scale_factor: f64) -> u32 {
    const SIZES: [u32; 15] = [16, 20, 24, 28, 32, 36, 40, 48, 56, 64, 72, 80, 96, 112, 128];
    let target = (f64::from(base_size) * scale_factor.clamp(1.0, 4.0)).round() as u32;

    SIZES
        .into_iter()
        .min_by_key(|size| size.abs_diff(target))
        .unwrap_or(base_size)
}

#[cfg(target_os = "windows")]
fn windows_icon(size: u32) -> Result<tauri::image::Image<'static>, Box<dyn std::error::Error>> {
    let bytes: &'static [u8] = match size {
        20 => include_bytes!("../../build/icons/20x20.png"),
        24 => include_bytes!("../../build/icons/24x24.png"),
        28 => include_bytes!("../../build/icons/28x28.png"),
        32 => include_bytes!("../../build/icons/32x32.png"),
        36 => include_bytes!("../../build/icons/36x36.png"),
        40 => include_bytes!("../../build/icons/40x40.png"),
        48 => include_bytes!("../../build/icons/48x48.png"),
        56 => include_bytes!("../../build/icons/56x56.png"),
        64 => include_bytes!("../../build/icons/64x64.png"),
        72 => include_bytes!("../../build/icons/72x72.png"),
        80 => include_bytes!("../../build/icons/80x80.png"),
        96 => include_bytes!("../../build/icons/96x96.png"),
        112 => include_bytes!("../../build/icons/112x112.png"),
        128 => include_bytes!("../../build/icons/128x128.png"),
        _ => include_bytes!("../../build/icons/16x16.png"),
    };
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info()?;
    let mut rgba = vec![0; reader.output_buffer_size()];
    let output = reader.next_frame(&mut rgba)?;
    if output.color_type != png::ColorType::Rgba || output.bit_depth != png::BitDepth::Eight {
        return Err("Windows icon must be an 8-bit RGBA PNG".into());
    }
    rgba.truncate(output.buffer_size());

    Ok(tauri::image::Image::new_owned(
        rgba,
        output.width,
        output.height,
    ))
}

#[cfg(target_os = "windows")]
fn set_windows_taskbar_icon(
    window: &tauri::WebviewWindow,
    image: &tauri::image::Image<'_>,
) -> Result<isize, Box<dyn std::error::Error>> {
    let mut bgra = image.rgba().to_vec();
    let mut and_mask = Vec::with_capacity(bgra.len() / 4);
    for pixel in bgra.chunks_exact_mut(4) {
        and_mask.push(pixel[3].wrapping_sub(u8::MAX));
        pixel.swap(0, 2);
    }

    let icon = unsafe {
        CreateIcon(
            None,
            image.width() as i32,
            image.height() as i32,
            1,
            32,
            and_mask.as_ptr(),
            bgra.as_ptr(),
        )
    }?;
    let hwnd = window.hwnd()?;
    unsafe {
        SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_BIG as usize)),
            Some(LPARAM(icon.0 as isize)),
        );
    }

    Ok(icon.0 as isize)
}

#[cfg(target_os = "windows")]
fn refresh_windows_window_icons(
    window: &tauri::WebviewWindow,
    scale_factor: f64,
) -> Result<(), Box<dyn std::error::Error>> {
    window.set_icon(windows_icon(windows_icon_size(16, scale_factor))?)?;
    let taskbar_icon = windows_icon(windows_icon_size(32, scale_factor))?;
    let taskbar_icon = set_windows_taskbar_icon(window, &taskbar_icon)?;
    let Some(state) = window.app_handle().try_state::<WindowsTaskbarIcon>() else {
        let _ = unsafe { DestroyIcon(HICON(taskbar_icon as _)) };
        return Err("Windows taskbar icon is not initialized".into());
    };
    state.replace(taskbar_icon);

    if let Ok(Some(monitor)) = window.app_handle().primary_monitor() {
        if let Some(tray) = window.app_handle().tray_by_id(TRAY_ID) {
            tray.set_icon(Some(windows_icon(windows_icon_size(
                16,
                monitor.scale_factor(),
            ))?))?;
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn paint_tray_icon_disc(rgba: &mut [u8], center_x: i32, center_y: i32, radius: i32) {
    const SIZE: i32 = 32;
    for y in (center_y - radius)..=(center_y + radius) {
        for x in (center_x - radius)..=(center_x + radius) {
            if x < 0
                || y < 0
                || x >= SIZE
                || y >= SIZE
                || (x - center_x).pow(2) + (y - center_y).pow(2) > radius.pow(2)
            {
                continue;
            }
            let pixel = (y as usize * SIZE as usize + x as usize) * 4;
            rgba[pixel + 3] = u8::MAX;
        }
    }
}

#[cfg(target_os = "macos")]
fn paint_tray_icon_segment(rgba: &mut [u8], start: (i32, i32), end: (i32, i32)) {
    let steps = (end.0 - start.0).abs().max((end.1 - start.1).abs());
    for step in 0..=steps {
        let x = start.0 + (end.0 - start.0) * step / steps;
        let y = start.1 + (end.1 - start.1) * step / steps;
        paint_tray_icon_disc(rgba, x, y, 1);
    }
}

#[cfg(target_os = "macos")]
fn paint_tray_icon_arc(
    rgba: &mut [u8],
    center: (i32, i32),
    radius: f32,
    start_degrees: f32,
    end_degrees: f32,
) {
    let sweep = (end_degrees - start_degrees).abs();
    let steps = ((sweep / 360.0 * 96.0).ceil() as i32).max(1);
    for step in 0..=steps {
        let progress = step as f32 / steps as f32;
        let angle = (start_degrees + (end_degrees - start_degrees) * progress).to_radians();
        let x = center.0 + (radius * angle.cos()).round() as i32;
        let y = center.1 + (radius * angle.sin()).round() as i32;
        paint_tray_icon_disc(rgba, x, y, 1);
    }
}

#[cfg(target_os = "macos")]
fn tray_template_icon() -> tauri::image::Image<'static> {
    const SIZE: usize = 32;
    let mut rgba = vec![0_u8; SIZE * SIZE * 4];

    for (start_degrees, end_degrees) in [(150.0, 210.0), (232.0, 308.0), (330.0, 390.0)] {
        paint_tray_icon_arc(&mut rgba, (16, 18), 11.0, start_degrees, end_degrees);
    }

    paint_tray_icon_segment(&mut rgba, (16, 19), (22, 13));
    paint_tray_icon_disc(&mut rgba, 16, 19, 2);

    tauri::image::Image::new_owned(rgba, SIZE as u32, SIZE as u32)
}

fn handle_run_event(app: &tauri::AppHandle, event: tauri::RunEvent) {
    #[cfg(target_os = "macos")]
    if matches!(event, tauri::RunEvent::Reopen { .. }) {
        show_main_window(app);
    }

    #[cfg(not(target_os = "macos"))]
    let _ = (app, event);
}

#[cfg(target_os = "windows")]
fn create_tray_status_window(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    if app.get_webview_window(TRAY_STATUS_WINDOW_LABEL).is_some() {
        return Ok(());
    }

    tauri::WebviewWindowBuilder::new(
        app,
        TRAY_STATUS_WINDOW_LABEL,
        tauri::WebviewUrl::App("tray.html".into()),
    )
    .title("Dial Status")
    .inner_size(320.0, 156.0)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .closable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false)
    .shadow(true)
    .build()?;
    Ok(())
}

fn setup_tray(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "windows")]
    create_tray_status_window(app)?;

    let settings = load_settings();
    let language = tray_language(&settings.language);
    let (open_label, quit_label) = tray_menu_labels(language);
    let open = MenuItem::with_id(app, TRAY_OPEN_MENU_ID, open_label, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, TRAY_QUIT_MENU_ID, quit_label, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;

    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .title("--%")
        .tooltip("Dial")
        .on_menu_event(|app, event| match event.id().as_ref() {
            TRAY_OPEN_MENU_ID => show_main_window(app),
            TRAY_QUIT_MENU_ID => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            #[cfg(target_os = "windows")]
            if let TrayIconEvent::Click {
                rect,
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let scale_factor = tray
                    .app_handle()
                    .get_webview_window(TRAY_STATUS_WINDOW_LABEL)
                    .and_then(|window| window.scale_factor().ok())
                    .unwrap_or(1.0);
                toggle_tray_status_window(
                    tray.app_handle(),
                    rect.position.to_physical(scale_factor),
                    rect.size.to_physical(scale_factor),
                );
            }

            #[cfg(not(target_os = "windows"))]
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    #[cfg(target_os = "macos")]
    {
        tray = tray.icon(tray_template_icon()).icon_as_template(true);
    }
    #[cfg(target_os = "windows")]
    {
        let tray_scale_factor = app
            .primary_monitor()?
            .map(|monitor| monitor.scale_factor())
            .unwrap_or(1.0);
        tray = tray.icon(windows_icon(windows_icon_size(16, tray_scale_factor))?);
        if let Some(window) = app.get_webview_window("main") {
            let window_scale_factor = window.scale_factor().unwrap_or(tray_scale_factor);
            window.set_icon(windows_icon(windows_icon_size(16, window_scale_factor))?)?;
            let taskbar_icon = windows_icon(windows_icon_size(32, window_scale_factor))?;
            let taskbar_icon = set_windows_taskbar_icon(&window, &taskbar_icon)?;
            if !app.manage(WindowsTaskbarIcon(Mutex::new(taskbar_icon))) {
                return Err("Windows taskbar icon is already initialized".into());
            }
        }
    }
    #[cfg(target_os = "linux")]
    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }
    tray.build(app)?;
    let display = TrayDisplayState {
        language,
        active_provider: settings.active_provider.clone(),
        remaining_percent: None,
        theme: settings.theme,
        accent_color: settings.accent_color,
        auto_refresh_enabled: settings.auto_refresh_enabled,
        auto_refresh_minutes: settings.auto_refresh_minutes,
        is_refreshing: false,
        refresh_failed: false,
        refresh_in_flight: false,
        pending_refresh_request_id: None,
        latest_refresh_request_id: 0,
    };
    let initial_display = display.clone();
    if !app.manage(TrayUiState {
        display: Mutex::new(display),
        auto_refresh_notify: Arc::new(tokio::sync::Notify::new()),
        stats_scan_lock: Arc::new(Mutex::new(())),
        scan_cache: Arc::new(ScanCacheStore::default()),
        scan_diagnostics: Arc::new(Mutex::new(HashMap::new())),
        open_item: open,
        quit_item: quit,
    }) {
        return Err("tray state is already initialized".into());
    }
    apply_tray_display(app.handle(), &initial_display);
    start_tray_auto_refresh(app.handle().clone());
    Ok(())
}

#[tauri::command]
fn sync_tray_language(app: tauri::AppHandle, language: String) -> Result<(), String> {
    let language = tray_language(&language);
    let Some(state) = app.try_state::<TrayUiState>() else {
        return Err("tray state is not initialized".to_string());
    };
    let (open_label, quit_label) = tray_menu_labels(language);
    state
        .open_item
        .set_text(open_label)
        .map_err(|error| error.to_string())?;
    state
        .quit_item
        .set_text(quit_label)
        .map_err(|error| error.to_string())?;
    let mut display = state
        .display
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    display.language = language;
    apply_tray_display(&app, &display);
    Ok(())
}

#[tauri::command]
fn get_settings() -> Settings {
    load_settings()
}

#[tauri::command]
fn update_settings(app: tauri::AppHandle, settings: Settings) -> Result<Settings, String> {
    let settings = normalize_settings(settings);
    save_settings(&settings)?;
    sync_tray_settings(&app, &settings);
    Ok(settings)
}

async fn run_stats_scan(
    app: tauri::AppHandle,
    provider: Option<String>,
    force: bool,
) -> Result<Stats, String> {
    let task_app = app.clone();
    let state = app
        .try_state::<TrayUiState>()
        .ok_or_else(|| "tray state is not initialized".to_string())?;
    let scan_lock = state.stats_scan_lock.clone();
    let scan_cache = state.scan_cache.clone();
    let scan_diagnostics = state.scan_diagnostics.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _scan = scan_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let settings = load_settings();
        let provider = provider.unwrap_or_else(|| settings.active_provider.clone());
        let request_id = begin_external_tray_refresh(&task_app, &provider);
        let result = read_stats_for_provider(&settings, &provider, &scan_cache, force);
        if let Ok(result) = &result {
            remember_scan_diagnostics(&scan_diagnostics, &result.diagnostics);
        }
        if let Some(request_id) = request_id {
            complete_external_tray_refresh(
                &task_app,
                request_id,
                &provider,
                result
                    .as_ref()
                    .map(|result| &result.stats)
                    .map_err(String::as_str),
            );
        }
        result.map(|result| (provider, result.stats))
    })
    .await
    .map_err(|error| error.to_string())?;
    let (_, stats) = result?;
    Ok(stats)
}

#[tauri::command]
async fn get_stats(app: tauri::AppHandle, provider: Option<String>) -> Result<Stats, String> {
    run_stats_scan(app, provider, false).await
}

#[tauri::command]
async fn rebuild_stats_cache(
    app: tauri::AppHandle,
    provider: Option<String>,
) -> Result<Stats, String> {
    run_stats_scan(app, provider, true).await
}

#[tauri::command]
fn get_scan_diagnostics(app: tauri::AppHandle) -> Result<Vec<ScanDiagnostics>, String> {
    let state = app
        .try_state::<TrayUiState>()
        .ok_or_else(|| "tray state is not initialized".to_string())?;
    let mut diagnostics = state
        .scan_diagnostics
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .values()
        .cloned()
        .collect::<Vec<_>>();
    diagnostics.sort_by(|left, right| left.provider.cmp(&right.provider));
    Ok(diagnostics)
}

#[tauri::command]
async fn choose_home(
    app: tauri::AppHandle,
    provider: String,
) -> Result<Option<ChooseHomeResult>, String> {
    let provider = match provider.as_str() {
        "claude" => "claude",
        "copilot" => "copilot",
        "cursor" => "cursor",
        "chatgpt" => "chatgpt",
        _ => "codex",
    }
    .to_string();
    let title = match provider.as_str() {
        "claude" => "Select Claude Code data directory",
        "copilot" => "Select GitHub Copilot data directory",
        "cursor" => "Select Cursor data directory",
        "chatgpt" => "Select ChatGPT data directory",
        _ => "Select Codex data directory",
    };

    let Some(folder) = rfd::FileDialog::new().set_title(title).pick_folder() else {
        return Ok(None);
    };

    let folder = folder.to_string_lossy().to_string();
    let task_app = app.clone();
    let state = app
        .try_state::<TrayUiState>()
        .ok_or_else(|| "tray state is not initialized".to_string())?;
    let scan_lock = state.stats_scan_lock.clone();
    let scan_cache = state.scan_cache.clone();
    let scan_diagnostics = state.scan_diagnostics.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _scan = scan_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut settings = load_settings();
        settings.active_provider = provider.clone();
        match provider.as_str() {
            "claude" => settings.claude_home = folder,
            "copilot" => settings.copilot_home = folder,
            "cursor" => settings.cursor_home = folder,
            "chatgpt" => settings.chatgpt_home = folder,
            _ => settings.codex_home = folder,
        }
        settings = normalize_settings(settings);
        save_settings(&settings)?;
        sync_tray_settings(&task_app, &settings);
        let request_id = begin_external_tray_refresh(&task_app, &provider);
        let stats_result = read_stats_for_provider(&settings, &provider, &scan_cache, false);
        if let Ok(result) = &stats_result {
            remember_scan_diagnostics(&scan_diagnostics, &result.diagnostics);
        }
        if let Some(request_id) = request_id {
            complete_external_tray_refresh(
                &task_app,
                request_id,
                &provider,
                stats_result
                    .as_ref()
                    .map(|result| &result.stats)
                    .map_err(String::as_str),
            );
        }
        let stats = stats_result?.stats;

        Ok::<_, String>(Some(ChooseHomeResult { settings, stats }))
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(result)
}

#[tauri::command]
fn start_window_drag(window: tauri::Window) -> Result<(), String> {
    window.start_dragging().map_err(|error| error.to_string())
}

#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    let allowed_urls = [
        "https://github.com/peipeitu/dial",
        "https://github.com/peipeitu/dial/issues",
    ];

    if !allowed_urls.contains(&url.as_str()) {
        return Err("URL is not allowed".to_string());
    }

    #[cfg(target_os = "macos")]
    let status = Command::new("open").arg(&url).status();

    #[cfg(target_os = "windows")]
    let status = Command::new("cmd").args(["/C", "start", "", &url]).status();

    #[cfg(all(unix, not(target_os = "macos")))]
    let status = Command::new("xdg-open").arg(&url).status();

    status
        .map_err(|error| error.to_string())
        .and_then(|status| {
            status
                .success()
                .then_some(())
                .ok_or_else(|| "Unable to open URL".to_string())
        })
}

#[tauri::command]
async fn check_update(app: tauri::AppHandle) -> Result<UpdateInfo, String> {
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = app;
        Ok(unsupported_update_info())
    }

    #[cfg(any(windows, target_os = "linux"))]
    {
        let Some(pubkey) = updater_public_key() else {
            return Ok(unsupported_update_info());
        };
        let endpoint = url::Url::parse(UPDATE_ENDPOINT).map_err(|error| error.to_string())?;
        let updater = app
            .updater_builder()
            .pubkey(pubkey)
            .endpoints(vec![endpoint])
            .map_err(|error| error.to_string())?
            .build()
            .map_err(|error| error.to_string())?;

        match updater.check().await.map_err(|error| error.to_string())? {
            Some(update) => Ok(UpdateInfo {
                supported: true,
                available: true,
                current_version: update.current_version,
                version: Some(update.version),
                notes: update.body,
                published_at: update.date.and_then(updater_published_at),
            }),
            None => Ok(UpdateInfo {
                supported: true,
                available: false,
                current_version: current_app_version(),
                version: None,
                notes: None,
                published_at: None,
            }),
        }
    }
}

#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = app;
        Err("Automatic updates are not supported on this platform.".to_string())
    }

    #[cfg(any(windows, target_os = "linux"))]
    {
        let Some(pubkey) = updater_public_key() else {
            return Err("Updater public key is not configured.".to_string());
        };
        let endpoint = url::Url::parse(UPDATE_ENDPOINT).map_err(|error| error.to_string())?;
        let updater = app
            .updater_builder()
            .pubkey(pubkey)
            .endpoints(vec![endpoint])
            .map_err(|error| error.to_string())?
            .build()
            .map_err(|error| error.to_string())?;

        let Some(update) = updater.check().await.map_err(|error| error.to_string())? else {
            return Ok(());
        };

        update
            .download_and_install(|_, _| {}, || {})
            .await
            .map_err(|error| error.to_string())?;
        app.restart();
    }
}

fn read_stats_for_provider(
    settings: &Settings,
    provider: &str,
    scan_cache: &ScanCacheStore,
    force: bool,
) -> Result<StatsScanResult, String> {
    match provider {
        "claude" => read_claude_stats(settings, scan_cache, force),
        "copilot" => read_copilot_stats(settings, scan_cache, force),
        "cursor" => read_cursor_stats(settings, scan_cache, force),
        "chatgpt" => read_chatgpt_stats(settings, scan_cache, force),
        _ => read_codex_stats(settings, scan_cache, force),
    }
}

fn iso_now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn iso_from_ms(ms: i64) -> Option<String> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|date| date.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn start_of_local_day_ms(now: DateTime<Local>) -> i64 {
    Local
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .single()
        .unwrap_or(now)
        .timestamp_millis()
}

fn local_date_key(ms: i64) -> Option<String> {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|date| date.format("%Y-%m-%d").to_string())
}

fn build_empty_daily_series(days: u32, now: DateTime<Local>) -> Vec<DailyPoint> {
    let today = Local
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .single()
        .unwrap_or(now);

    (0..days)
        .map(|index| {
            let date = today - Duration::days((days - index - 1) as i64);
            DailyPoint {
                date: date.format("%Y-%m-%d").to_string(),
                label: format!("{}/{}", date.month(), date.day()),
                threads: 0,
                tokens: 0,
                cost: 0.0,
            }
        })
        .collect()
}

fn rank_by_tokens(items: impl IntoIterator<Item = (String, u64)>, limit: usize) -> Vec<RankItem> {
    let mut totals: HashMap<String, u64> = HashMap::new();
    for (name, value) in items {
        *totals.entry(name).or_default() += value;
    }

    let mut items: Vec<_> = totals
        .into_iter()
        .map(|(name, value)| RankItem { name, value })
        .collect();
    items.sort_by_key(|item| std::cmp::Reverse(item.value));
    items.truncate(limit);
    items
}

fn rank_by_tokens_with_other(
    items: impl IntoIterator<Item = (String, u64)>,
    visible_limit: usize,
    other_name: &str,
) -> Vec<RankItem> {
    let mut ranked = rank_by_tokens(items, usize::MAX);
    if ranked.len() <= visible_limit {
        return ranked;
    }

    let other_value = ranked
        .iter()
        .skip(visible_limit)
        .map(|item| item.value)
        .sum();
    ranked.truncate(visible_limit);
    ranked.push(RankItem {
        name: other_name.to_string(),
        value: other_value,
    });
    ranked
}

fn thread_usage_total(thread: &Thread) -> u64 {
    let event_total: u64 = thread
        .usage_events
        .iter()
        .map(|event| event.total_tokens)
        .sum();
    if event_total > 0 {
        event_total
    } else {
        thread.tokens_used
    }
}

fn format_plan_type(plan_type: Option<&str>) -> String {
    let Some(plan_type) = plan_type else {
        return "Codex".to_string();
    };

    let normalized = plan_type.trim().to_lowercase();
    if normalized == "prolite" {
        return "Pro".to_string();
    }

    let mut chars = normalized.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "Codex".to_string(),
    }
}

fn plan_monthly_usd(plan_type: Option<&str>) -> Option<f64> {
    match plan_type.unwrap_or("").trim().to_lowercase().as_str() {
        "free" => Some(0.0),
        "go" => Some(8.0),
        "plus" => Some(20.0),
        "pro" | "prolite" => Some(100.0),
        "business" | "team" => Some(20.0),
        _ => None,
    }
}

fn initials_from_name(name: &str, fallback: &str) -> String {
    let initials: String = name
        .split_whitespace()
        .filter_map(|part| part.chars().next())
        .take(2)
        .flat_map(|character| character.to_uppercase())
        .collect();

    if initials.is_empty() {
        fallback.to_string()
    } else {
        initials
    }
}

fn decode_jwt_payload(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let decoded = general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| general_purpose::URL_SAFE.decode(payload))
        .ok()?;
    serde_json::from_slice(&decoded).ok()
}

fn read_codex_account(codex_home: &Path, latest_plan_type: Option<&str>) -> Account {
    let auth_path = codex_home.join("auth.json");
    let claims = fs::read_to_string(auth_path)
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
        .and_then(|auth| {
            auth.pointer("/tokens/id_token")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .and_then(|token| decode_jwt_payload(&token));

    let display_name = claims
        .as_ref()
        .and_then(|claims| {
            claims
                .get("name")
                .or_else(|| claims.get("nickname"))
                .or_else(|| claims.get("email"))
                .and_then(Value::as_str)
        })
        .unwrap_or("Codex")
        .to_string();

    Account {
        initials: initials_from_name(&display_name, "CD"),
        display_name,
        plan_type: latest_plan_type.map(str::to_string),
        plan_label: format_plan_type(latest_plan_type),
        plan_monthly_usd: plan_monthly_usd(latest_plan_type),
    }
}

fn source_label(value: Option<&str>) -> String {
    let Some(value) = value else {
        return "Unknown".to_string();
    };

    if let Ok(parsed) = serde_json::from_str::<Value>(value) {
        if parsed.get("subagent").is_some() {
            return "子任务".to_string();
        }
    }

    if value.eq_ignore_ascii_case("vscode") {
        "VS Code".to_string()
    } else if value.is_empty() {
        "Unknown".to_string()
    } else {
        value.to_string()
    }
}

fn codex_home(settings: &Settings) -> PathBuf {
    if !settings.codex_home.trim().is_empty() {
        return PathBuf::from(&settings.codex_home);
    }
    if let Ok(value) = env::var("CODEX_HOME") {
        if !value.trim().is_empty() {
            return PathBuf::from(value);
        }
    }
    home_dir().join(".codex")
}

fn first_existing(paths: &[PathBuf]) -> PathBuf {
    paths
        .iter()
        .find(|path| path.exists())
        .cloned()
        .unwrap_or_else(|| paths[0].clone())
}

fn codex_paths(settings: &Settings) -> (PathBuf, PathBuf, Value) {
    let home = codex_home(settings);
    let state_db = first_existing(&[
        home.join("state_5.sqlite"),
        home.join("sqlite").join("state_5.sqlite"),
    ]);
    let logs_db = first_existing(&[
        home.join("logs_2.sqlite"),
        home.join("sqlite").join("logs_2.sqlite"),
    ]);

    let paths = json!({
      "codexHome": home.to_string_lossy(),
      "stateDbPath": state_db.to_string_lossy(),
      "logsDbPath": logs_db.to_string_lossy(),
      "sessionIndexPath": home.join("session_index.jsonl").to_string_lossy()
    });

    (home, state_db, paths)
}

fn sqlite_nonnegative_u64(value: Option<i64>) -> u64 {
    value
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(0)
}

fn read_codex_threads(db_path: &Path) -> Result<Vec<Thread>, String> {
    let connection = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| error.to_string())?;
    let mut statement = connection
    .prepare(
      "select id, title, source, model_provider, model, cwd, archived, tokens_used, rollout_path, created_at, updated_at, created_at_ms, updated_at_ms, preview from threads",
    )
    .map_err(|error| error.to_string())?;

    let rows = statement
        .query_map([], |row| {
            let created_at_ms: Option<i64> = row.get(11)?;
            let updated_at_ms: Option<i64> = row.get(12)?;
            let created_at: Option<i64> = row.get(9)?;
            let updated_at: Option<i64> = row.get(10)?;
            let created = created_at_ms.unwrap_or_else(|| created_at.unwrap_or(0) * 1000);
            let updated =
                updated_at_ms.unwrap_or_else(|| updated_at.unwrap_or(created / 1000) * 1000);
            let source: Option<String> = row.get(2)?;

            Ok(Thread {
                id: row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                title: row
                    .get::<_, Option<String>>(1)?
                    .unwrap_or_else(|| "Untitled".to_string()),
                source: source_label(source.as_deref()),
                model: row
                    .get::<_, Option<String>>(4)?
                    .unwrap_or_else(|| "Unknown".to_string()),
                cwd: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                archived: row.get::<_, Option<i64>>(6)?.unwrap_or(0) == 1,
                tokens_used: sqlite_nonnegative_u64(row.get::<_, Option<i64>>(7)?),
                rollout_path: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
                created_at_ms: created,
                updated_at_ms: updated,
                usage_events: Vec::new(),
            })
        })
        .map_err(|error| error.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn rate_limit_window_label(minutes: u64) -> String {
    if minutes == 300 {
        "5 小时".to_string()
    } else if minutes == 10080 {
        "1 周".to_string()
    } else if minutes >= 10080 && minutes.is_multiple_of(10080) {
        format!("{} 周", minutes / 10080)
    } else if minutes >= 1440 && minutes.is_multiple_of(1440) {
        format!("{} 天", minutes / 1440)
    } else if minutes >= 60 && minutes.is_multiple_of(60) {
        format!("{} 小时", minutes / 60)
    } else {
        format!("{minutes} 分钟")
    }
}

fn normalize_rate_limit_window(id: &str, window: Option<&Value>) -> Option<RateLimitWindow> {
    let window = window?;
    let used_percent = window
        .get("used_percent")
        .and_then(Value::as_f64)
        .filter(|percent| percent.is_finite())?
        .clamp(0.0, 100.0);
    let window_minutes = window
        .get("window_minutes")
        .and_then(Value::as_u64)
        .filter(|minutes| *minutes > 0)?;
    let resets_at = window
        .get("resets_at")
        .and_then(Value::as_i64)
        .and_then(|seconds| iso_from_ms(seconds * 1000));

    Some(RateLimitWindow {
        id: id.to_string(),
        label: rate_limit_window_label(window_minutes),
        used_percent,
        remaining_percent: (100.0 - used_percent).max(0.0),
        window_minutes,
        resets_at,
    })
}

fn normalize_rate_limits(rate_limits: Option<&Value>, timestamp_ms: i64) -> Option<RateLimits> {
    let rate_limits = rate_limits?;
    let windows: Vec<_> = [
        normalize_rate_limit_window("primary", rate_limits.get("primary")),
        normalize_rate_limit_window("secondary", rate_limits.get("secondary")),
    ]
    .into_iter()
    .flatten()
    .collect();

    if windows.is_empty() {
        return None;
    }

    Some(RateLimits {
        updated_at: iso_from_ms(timestamp_ms),
        plan_type: rate_limits
            .get("plan_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        reached_type: rate_limits
            .get("rate_limit_reached_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        windows,
    })
}

fn build_rate_limit_history(
    usage_events: &[UsageEvent],
    now_ms: i64,
) -> Vec<RateLimitHistorySeries> {
    let mut grouped = RateLimitHistoryGroups::new();

    for event in usage_events {
        let Some(rate_limits) = event.rate_limits.as_ref() else {
            continue;
        };

        for window in &rate_limits.windows {
            let window_ms = window.window_minutes as i64 * 60 * 1000;
            if window_ms <= 0 || event.timestamp_ms < now_ms - window_ms {
                continue;
            }

            grouped
                .entry((window.id.clone(), window.window_minutes))
                .or_insert_with(|| (window.label.clone(), Vec::new()))
                .1
                .push((
                    event.timestamp_ms,
                    window.used_percent,
                    window.remaining_percent,
                ));
        }
    }

    let mut series = grouped
        .into_iter()
        .map(|((id, window_minutes), (label, mut points))| {
            points.sort_by_key(|point| point.0);
            points.dedup_by_key(|point| point.0);
            if points.len() > 512 {
                points.drain(0..points.len() - 512);
            }

            RateLimitHistorySeries {
                id,
                label,
                window_minutes,
                points: points
                    .into_iter()
                    .filter_map(|(timestamp_ms, used_percent, remaining_percent)| {
                        Some(RateLimitHistoryPoint {
                            timestamp: iso_from_ms(timestamp_ms)?,
                            used_percent,
                            remaining_percent,
                        })
                    })
                    .collect(),
            }
        })
        .collect::<Vec<_>>();

    series.retain(|item| !item.points.is_empty());
    series.sort_by_key(|item| item.window_minutes);
    series
}

fn read_codex_usage_events(threads: &[Thread]) -> Vec<UsageEvent> {
    let mut events = Vec::new();

    for thread in threads {
        if thread.rollout_path.is_empty() {
            continue;
        }

        let path = Path::new(&thread.rollout_path);
        if !path.exists() {
            continue;
        }

        let Ok(file) = File::open(path) else {
            continue;
        };

        let mut previous_total_usage: Option<u64> = None;
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if !line.contains("\"token_count\"") {
                continue;
            }

            let Ok(entry) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if entry.get("type").and_then(Value::as_str) != Some("event_msg")
                || entry.pointer("/payload/type").and_then(Value::as_str) != Some("token_count")
            {
                continue;
            }

            let timestamp_ms = entry
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(|timestamp| DateTime::parse_from_rfc3339(timestamp).ok())
                .map(|timestamp| timestamp.timestamp_millis());
            let Some(timestamp_ms) = timestamp_ms else {
                continue;
            };

            let last_total_tokens = entry
                .pointer("/payload/info/last_token_usage/total_tokens")
                .and_then(Value::as_u64);
            let cumulative_total_tokens = entry
                .pointer("/payload/info/total_token_usage/total_tokens")
                .and_then(Value::as_u64);
            let total_tokens = match cumulative_total_tokens {
                Some(cumulative) => {
                    let delta = previous_total_usage
                        .map(|previous| cumulative.saturating_sub(previous))
                        .unwrap_or_else(|| last_total_tokens.unwrap_or(cumulative));
                    previous_total_usage = Some(cumulative);
                    delta
                }
                None => last_total_tokens.unwrap_or(0),
            };
            let rate_limits =
                normalize_rate_limits(entry.pointer("/payload/rate_limits"), timestamp_ms);
            if total_tokens == 0 && rate_limits.is_none() {
                continue;
            }

            events.push(UsageEvent {
                thread_id: thread.id.clone(),
                timestamp_ms,
                model: thread.model.clone(),
                total_tokens,
                plan_type: entry
                    .pointer("/payload/rate_limits/plan_type")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                rate_limits,
            });
        }
    }

    events
}

fn bind_codex_usage_events(
    scanned_files: Vec<ScannedFile<UsageEvent>>,
    rollout_threads: &HashMap<PathBuf, Vec<Thread>>,
) -> Vec<UsageEvent> {
    let mut usage_events = Vec::new();
    for scanned_file in scanned_files {
        let Some(threads) = rollout_threads.get(&scanned_file.path) else {
            continue;
        };
        for thread in threads {
            usage_events.extend(scanned_file.values.iter().cloned().map(|mut event| {
                event.thread_id = thread.id.clone();
                event.model = thread.model.clone();
                event
            }));
        }
    }
    usage_events
}

#[allow(clippy::too_many_arguments)]
fn build_stats_from_threads(
    mut threads: Vec<Thread>,
    now: DateTime<Local>,
    chart_days: u32,
    account: Account,
    pricing: Option<Pricing>,
    usd_per_million_tokens: Option<f64>,
    rate_limits_enabled: bool,
    paths: Value,
) -> Stats {
    let usage_events: Vec<UsageEvent> = threads
        .iter()
        .flat_map(|thread| thread.usage_events.clone())
        .collect();
    let has_usage_events = usage_events.iter().any(|event| event.total_tokens > 0);
    let today_start_ms = start_of_local_day_ms(now);
    let period_start_ms = today_start_ms - (chart_days as i64 - 1) * 24 * 60 * 60 * 1000;
    let recent_threshold = now.timestamp_millis() - 7 * 24 * 60 * 60 * 1000;
    let active_threads = threads.iter().filter(|thread| !thread.archived).count();
    let total_tokens = threads.iter().map(thread_usage_total).sum();
    let updated_this_week = threads
        .iter()
        .filter(|thread| thread.updated_at_ms >= recent_threshold)
        .count();

    let period_events: Vec<_> = usage_events
        .iter()
        .filter(|event| event.timestamp_ms >= period_start_ms)
        .collect();
    let today_events: Vec<_> = usage_events
        .iter()
        .filter(|event| event.timestamp_ms >= today_start_ms)
        .collect();
    let today_tokens = if has_usage_events {
        today_events.iter().map(|event| event.total_tokens).sum()
    } else {
        threads
            .iter()
            .filter(|thread| thread.created_at_ms >= today_start_ms)
            .map(|thread| thread.tokens_used)
            .sum()
    };
    let period_tokens = if has_usage_events {
        period_events.iter().map(|event| event.total_tokens).sum()
    } else {
        threads
            .iter()
            .filter(|thread| thread.created_at_ms >= period_start_ms)
            .map(|thread| thread.tokens_used)
            .sum()
    };
    let cost_available = usd_per_million_tokens
        .map(|cost| cost.is_finite() && cost > 0.0)
        .unwrap_or(false);
    let estimate_provider_cost = |tokens: u64| {
        usd_per_million_tokens
            .filter(|cost| cost.is_finite() && *cost > 0.0)
            .map(|cost| (tokens as f64 / 1_000_000.0) * cost)
            .unwrap_or(0.0)
    };
    let period_cost = estimate_provider_cost(period_tokens);
    let latest_plan_type = usage_events
        .iter()
        .filter(|event| event.plan_type.is_some())
        .max_by_key(|event| event.timestamp_ms)
        .and_then(|event| event.plan_type.clone());
    let latest_rate_limits = if rate_limits_enabled {
        usage_events
            .iter()
            .filter(|event| event.rate_limits.is_some())
            .max_by_key(|event| event.timestamp_ms)
            .and_then(|event| event.rate_limits.clone())
    } else {
        None
    };
    let rate_limit_history = if rate_limits_enabled {
        build_rate_limit_history(&usage_events, now.timestamp_millis())
    } else {
        Vec::new()
    };

    let mut daily_series = build_empty_daily_series(chart_days, now);
    let mut day_indexes = HashMap::new();
    for (index, day) in daily_series.iter().enumerate() {
        day_indexes.insert(day.date.clone(), index);
    }

    for thread in &threads {
        if let Some(key) = local_date_key(thread.created_at_ms) {
            if let Some(index) = day_indexes.get(&key) {
                daily_series[*index].threads += 1;
                if !has_usage_events {
                    daily_series[*index].tokens += thread.tokens_used;
                    daily_series[*index].cost = estimate_provider_cost(daily_series[*index].tokens);
                }
            }
        }
    }

    for event in period_events {
        if let Some(key) = local_date_key(event.timestamp_ms) {
            if let Some(index) = day_indexes.get(&key) {
                daily_series[*index].tokens += event.total_tokens;
                daily_series[*index].cost = estimate_provider_cost(daily_series[*index].tokens);
            }
        }
    }

    threads.sort_by_key(|thread| std::cmp::Reverse(thread.updated_at_ms));
    let latest_threads = threads
        .iter()
        .take(8)
        .map(|thread| LatestThread {
            id: thread.id.clone(),
            title: thread.title.clone(),
            model: thread.model.clone(),
            source: thread.source.clone(),
            tokens_used: thread_usage_total(thread),
            updated_at: iso_from_ms(thread.updated_at_ms),
            cwd: thread.cwd.clone(),
        })
        .collect();
    let latest_token_usage = if today_tokens > 0 {
        today_tokens
    } else if has_usage_events {
        usage_events
            .iter()
            .max_by_key(|event| event.timestamp_ms)
            .map(|event| event.total_tokens)
            .unwrap_or(0)
    } else {
        threads
            .first()
            .map(|thread| thread.tokens_used)
            .unwrap_or(0)
    };

    let account = if latest_plan_type.is_some() && account.plan_type.is_none() {
        Account {
            plan_type: latest_plan_type.clone(),
            plan_label: format_plan_type(latest_plan_type.as_deref()),
            plan_monthly_usd: plan_monthly_usd(latest_plan_type.as_deref()),
            ..account
        }
    } else {
        account
    };

    Stats {
        generated_at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        error: None,
        account,
        settings: ChartSettings { chart_days },
        pricing,
        featured: Featured {
            today_tokens,
            today_cost: estimate_provider_cost(today_tokens),
            period_cost,
            period_tokens,
            latest_token_usage,
            cost_available,
            cost_estimated_from_token_events: has_usage_events,
        },
        rate_limits: latest_rate_limits,
        rate_limit_history,
        activity: None,
        totals: Totals {
            threads: threads.len(),
            active_threads,
            archived_threads: threads.len() - active_threads,
            total_tokens,
            updated_this_week,
        },
        daily_series,
        models: rank_by_tokens_with_other(
            threads
                .iter()
                .map(|thread| (thread.model.clone(), thread_usage_total(thread))),
            4,
            "其他",
        ),
        sources: rank_by_tokens(threads.iter().map(|thread| (thread.source.clone(), 1)), 6),
        workspaces: rank_by_tokens(
            threads
                .iter()
                .filter(|thread| !thread.cwd.is_empty())
                .map(|thread| {
                    let name = Path::new(&thread.cwd)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(&thread.cwd)
                        .to_string();
                    (name, thread_usage_total(thread))
                }),
            6,
        ),
        latest_threads,
        paths,
    }
}

fn empty_stats(
    provider_name: &str,
    initials: &str,
    chart_days: u32,
    error: String,
    paths: Value,
) -> Stats {
    Stats {
        generated_at: iso_now(),
        error: Some(error),
        account: Account {
            display_name: provider_name.to_string(),
            initials: initials.to_string(),
            plan_type: None,
            plan_label: provider_name.to_string(),
            plan_monthly_usd: None,
        },
        settings: ChartSettings { chart_days },
        pricing: None,
        featured: Featured {
            today_tokens: 0,
            today_cost: 0.0,
            period_cost: 0.0,
            period_tokens: 0,
            latest_token_usage: 0,
            cost_available: false,
            cost_estimated_from_token_events: true,
        },
        rate_limits: None,
        rate_limit_history: Vec::new(),
        activity: None,
        totals: Totals {
            threads: 0,
            active_threads: 0,
            archived_threads: 0,
            total_tokens: 0,
            updated_this_week: 0,
        },
        daily_series: build_empty_daily_series(chart_days, Local::now()),
        models: Vec::new(),
        sources: Vec::new(),
        workspaces: Vec::new(),
        latest_threads: Vec::new(),
        paths,
    }
}

fn codex_usage_history_key(home: &Path) -> String {
    home.to_string_lossy().to_string()
}

fn apply_codex_usage_history(
    history: &mut UsageHistory,
    home: &Path,
    stats: &mut Stats,
    usd_per_million_tokens: f64,
) -> bool {
    if let Some(paths) = stats.paths.as_object_mut() {
        paths.insert(
            "usageHistoryPath".to_string(),
            Value::String(usage_history_path().to_string_lossy().to_string()),
        );
    }

    let key = codex_usage_history_key(home);
    let daily_history = history.codex.entry(key).or_default();
    let mut changed = false;

    for day in &stats.daily_series {
        if day.tokens == 0 {
            continue;
        }

        let previous_tokens = daily_history
            .daily_tokens
            .get(&day.date)
            .copied()
            .unwrap_or(0);
        if day.tokens > previous_tokens {
            daily_history
                .daily_tokens
                .insert(day.date.clone(), day.tokens);
            changed = true;
        }
    }

    let estimate_cost = |tokens: u64| (tokens as f64 / 1_000_000.0) * usd_per_million_tokens;
    for day in &mut stats.daily_series {
        if let Some(snapshot_tokens) = daily_history.daily_tokens.get(&day.date) {
            day.tokens = day.tokens.max(*snapshot_tokens);
        }
        day.cost = estimate_cost(day.tokens);
    }

    let period_tokens: u64 = stats.daily_series.iter().map(|day| day.tokens).sum();
    let today_tokens = stats.daily_series.last().map(|day| day.tokens).unwrap_or(0);
    let latest_daily_tokens = stats
        .daily_series
        .iter()
        .rev()
        .find(|day| day.tokens > 0)
        .map(|day| day.tokens)
        .unwrap_or(0);
    let history_total: u64 = daily_history.daily_tokens.values().sum();

    stats.featured.today_tokens = today_tokens;
    stats.featured.today_cost = estimate_cost(today_tokens);
    stats.featured.period_tokens = period_tokens;
    stats.featured.period_cost = estimate_cost(period_tokens);
    stats.featured.latest_token_usage = if today_tokens > 0 {
        today_tokens
    } else {
        stats.featured.latest_token_usage.max(latest_daily_tokens)
    };
    stats.featured.cost_available =
        usd_per_million_tokens.is_finite() && usd_per_million_tokens > 0.0;
    if period_tokens > 0 {
        stats.featured.cost_estimated_from_token_events = true;
    }
    stats.totals.total_tokens = stats.totals.total_tokens.max(history_total);

    changed
}

fn codex_pricing() -> Pricing {
    Pricing {
        label: "Codex local log estimate".to_string(),
        url: Some("https://developers.openai.com/codex/pricing".to_string()),
        checked_at: "2026-06-30".to_string(),
    }
}

fn read_codex_stats(
    settings: &Settings,
    scan_cache: &ScanCacheStore,
    force: bool,
) -> Result<StatsScanResult, String> {
    let started_at = Instant::now();
    let chart_days = settings.chart_days.clamp(MIN_CHART_DAYS, MAX_CHART_DAYS);
    let (home, state_db, paths) = codex_paths(settings);

    if !state_db.exists() {
        let mut stats = empty_stats(
            "Codex",
            "CD",
            chart_days,
            format!(
                "Codex state database not found at {}",
                state_db.to_string_lossy()
            ),
            paths,
        );
        stats.account = read_codex_account(&home, None);
        stats.pricing = Some(codex_pricing());
        let mut history = load_usage_history();
        let changed = apply_codex_usage_history(
            &mut history,
            &home,
            &mut stats,
            CODEX_USD_PER_MILLION_TOKENS,
        );
        if changed {
            let _ = save_usage_history(&history);
        }
        return Ok(finish_scan(
            "codex",
            started_at,
            stats,
            ScanDiagnostics::empty("codex", force),
        ));
    }

    let mut threads = read_codex_threads(&state_db)?;
    let mut rollout_threads = HashMap::<PathBuf, Vec<Thread>>::new();
    for thread in threads
        .iter()
        .filter(|thread| !thread.rollout_path.is_empty())
    {
        rollout_threads
            .entry(PathBuf::from(&thread.rollout_path))
            .or_default()
            .push(thread.clone());
    }
    let rollout_files = rollout_threads.keys().cloned().collect::<Vec<_>>();
    let outcome = scan_cache.scan(
        scan_request(
            "codex",
            "codex-rollout-v1",
            std::slice::from_ref(&home),
            rollout_files,
            force,
        ),
        |path| {
            File::open(path).map_err(|error| error.to_string())?;
            let thread = rollout_threads
                .get(path)
                .and_then(|threads| threads.first())
                .ok_or_else(|| "Codex rollout no longer belongs to a known thread".to_string())?;
            Ok(read_codex_usage_events(std::slice::from_ref(thread)))
        },
    );
    let diagnostics = outcome.diagnostics.clone();
    let usage_events = bind_codex_usage_events(outcome.files, &rollout_threads);
    let mut usage_by_thread: HashMap<String, Vec<UsageEvent>> = HashMap::new();
    for event in usage_events {
        usage_by_thread
            .entry(event.thread_id.clone())
            .or_default()
            .push(event);
    }
    for thread in &mut threads {
        thread.usage_events = usage_by_thread.remove(&thread.id).unwrap_or_default();
    }

    let latest_plan_type = threads
        .iter()
        .flat_map(|thread| thread.usage_events.iter())
        .filter(|event| event.plan_type.is_some())
        .max_by_key(|event| event.timestamp_ms)
        .and_then(|event| event.plan_type.as_deref());
    let account = read_codex_account(&home, latest_plan_type);
    let mut stats = build_stats_from_threads(
        threads,
        Local::now(),
        chart_days,
        account,
        Some(codex_pricing()),
        Some(CODEX_USD_PER_MILLION_TOKENS),
        true,
        paths,
    );
    let mut history = load_usage_history();
    let changed = apply_codex_usage_history(
        &mut history,
        &home,
        &mut stats,
        CODEX_USD_PER_MILLION_TOKENS,
    );
    if changed {
        let _ = save_usage_history(&history);
    }

    Ok(finish_scan("codex", started_at, stats, diagnostics))
}

fn claude_home(settings: &Settings) -> PathBuf {
    if !settings.claude_home.trim().is_empty() {
        return PathBuf::from(&settings.claude_home);
    }
    for key in ["CLAUDE_CONFIG_DIR", "CLAUDE_HOME"] {
        if let Ok(value) = env::var(key) {
            if !value.trim().is_empty() {
                return PathBuf::from(value);
            }
        }
    }
    home_dir().join(".claude")
}

fn usage_total(usage: &Value) -> u64 {
    usage
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        + usage
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0)
        + usage
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0)
        + usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0)
}

fn normalize_claude_source(value: Option<&str>) -> String {
    match value.unwrap_or("") {
        value if value.eq_ignore_ascii_case("cli") => "CLI".to_string(),
        value if value.eq_ignore_ascii_case("vscode") => "VS Code".to_string(),
        "" => "Claude Code".to_string(),
        value => value.to_string(),
    }
}

fn read_claude_session(path: &Path) -> Option<Thread> {
    let file = File::open(path).ok()?;
    let metadata = fs::metadata(path).ok();
    let mut id = path.file_stem()?.to_string_lossy().to_string();
    let mut title = String::new();
    let mut cwd = String::new();
    let mut source = String::new();
    let mut sidechain = false;
    let mut created_at_ms: Option<i64> = None;
    let mut updated_at_ms: Option<i64> = None;
    let mut usage_by_message: HashMap<String, UsageEvent> = HashMap::new();
    let mut model_totals: HashMap<String, u64> = HashMap::new();

    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        if let Some(session_id) = entry.get("sessionId").and_then(Value::as_str) {
            id = session_id.to_string();
        }
        if title.is_empty() {
            if let Some(custom_title) = entry.get("customTitle").and_then(Value::as_str) {
                title = custom_title.to_string();
            } else if let Some(ai_title) = entry.get("aiTitle").and_then(Value::as_str) {
                title = ai_title.to_string();
            }
        }
        if cwd.is_empty() {
            if let Some(value) = entry.get("cwd").and_then(Value::as_str) {
                cwd = value.to_string();
            }
        }
        if source.is_empty() {
            if let Some(source_value) = entry
                .get("entrypoint")
                .or_else(|| entry.get("promptSource"))
                .and_then(Value::as_str)
            {
                source = normalize_claude_source(Some(source_value));
            }
        }
        if entry
            .get("isSidechain")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            sidechain = true;
        }

        let timestamp_ms = entry
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|timestamp| DateTime::parse_from_rfc3339(timestamp).ok())
            .map(|timestamp| timestamp.timestamp_millis());
        if let Some(timestamp_ms) = timestamp_ms {
            created_at_ms =
                Some(created_at_ms.map_or(timestamp_ms, |value| value.min(timestamp_ms)));
            updated_at_ms =
                Some(updated_at_ms.map_or(timestamp_ms, |value| value.max(timestamp_ms)));
        }

        if entry.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(usage) = entry.pointer("/message/usage") else {
            continue;
        };
        let total_tokens = usage_total(usage);
        if total_tokens == 0 {
            continue;
        }

        let message_id = entry
            .pointer("/message/id")
            .and_then(Value::as_str)
            .or_else(|| entry.get("uuid").and_then(Value::as_str))
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "{}:{}:{}",
                    id,
                    timestamp_ms.unwrap_or(0),
                    usage_by_message.len()
                )
            });
        let model = entry
            .pointer("/message/model")
            .and_then(Value::as_str)
            .unwrap_or("Unknown")
            .to_string();
        let event = UsageEvent {
            thread_id: id.clone(),
            timestamp_ms: timestamp_ms.unwrap_or_else(|| updated_at_ms.unwrap_or(0)),
            model: model.clone(),
            total_tokens,
            plan_type: None,
            rate_limits: None,
        };

        let should_update = usage_by_message
            .get(&message_id)
            .map(|previous| event.timestamp_ms >= previous.timestamp_ms)
            .unwrap_or(true);
        if should_update {
            usage_by_message.insert(message_id, event);
        }
    }

    let usage_events: Vec<_> = usage_by_message.into_values().collect();
    for event in &usage_events {
        *model_totals.entry(event.model.clone()).or_default() += event.total_tokens;
    }
    let tokens_used = usage_events.iter().map(|event| event.total_tokens).sum();
    let model = model_totals
        .into_iter()
        .max_by_key(|(_, tokens)| *tokens)
        .map(|(model, _)| model)
        .unwrap_or_else(|| "Unknown".to_string());
    let fallback_ms = metadata
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0);

    Some(Thread {
        id: id.clone(),
        title: if title.is_empty() {
            format!("Claude session {}", id.chars().take(8).collect::<String>())
        } else {
            title
        },
        source: if sidechain && source.is_empty() {
            "子任务".to_string()
        } else if source.is_empty() {
            "Claude Code".to_string()
        } else {
            source
        },
        model,
        cwd,
        archived: false,
        tokens_used,
        created_at_ms: created_at_ms.unwrap_or(fallback_ms),
        updated_at_ms: updated_at_ms.unwrap_or(fallback_ms),
        rollout_path: path.to_string_lossy().to_string(),
        usage_events,
    })
}

fn claude_estimated_token_limit(env_key: &str, fallback: u64) -> u64 {
    env::var(env_key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}

fn estimated_rate_limit_window(
    id: &str,
    label: String,
    window_minutes: u64,
    used_tokens: u64,
    token_limit: u64,
    resets_at_ms: Option<i64>,
) -> RateLimitWindow {
    let used_percent = if token_limit > 0 {
        ((used_tokens as f64 / token_limit as f64) * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };

    RateLimitWindow {
        id: id.to_string(),
        label,
        used_percent,
        remaining_percent: (100.0 - used_percent).max(0.0),
        window_minutes,
        resets_at: resets_at_ms.and_then(iso_from_ms),
    }
}

fn build_claude_estimated_rate_limits(
    threads: &[Thread],
    now: DateTime<Local>,
) -> Option<RateLimits> {
    if threads.is_empty() {
        return None;
    }

    let now_ms = now.timestamp_millis();
    let windows = [
        (
            "primary",
            300_u64,
            claude_estimated_token_limit(
                "AI_USAGE_CLAUDE_5H_TOKEN_LIMIT",
                CLAUDE_ESTIMATED_5H_TOKEN_LIMIT,
            ),
        ),
        (
            "secondary",
            10080_u64,
            claude_estimated_token_limit(
                "AI_USAGE_CLAUDE_WEEKLY_TOKEN_LIMIT",
                CLAUDE_ESTIMATED_WEEKLY_TOKEN_LIMIT,
            ),
        ),
    ]
    .into_iter()
    .map(|(id, window_minutes, token_limit)| {
        let window_start_ms = now_ms - window_minutes as i64 * 60 * 1000;
        let events = threads
            .iter()
            .flat_map(|thread| thread.usage_events.iter())
            .filter(|event| event.timestamp_ms >= window_start_ms && event.timestamp_ms <= now_ms)
            .collect::<Vec<_>>();
        let used_tokens = events.iter().map(|event| event.total_tokens).sum();
        let resets_at_ms = events
            .iter()
            .map(|event| event.timestamp_ms)
            .min()
            .map(|timestamp_ms| timestamp_ms + window_minutes as i64 * 60 * 1000);

        estimated_rate_limit_window(
            id,
            rate_limit_window_label(window_minutes),
            window_minutes,
            used_tokens,
            token_limit,
            resets_at_ms,
        )
    })
    .collect();

    Some(RateLimits {
        updated_at: Some(iso_now()),
        plan_type: Some("local_estimate".to_string()),
        reached_type: None,
        windows,
    })
}

fn find_claude_plan_type(value: &Value) -> Option<String> {
    find_string_deep_ci(
        value,
        &[
            "plan",
            "planType",
            "plan_type",
            "planName",
            "plan_name",
            "subscription",
            "subscriptionType",
            "subscription_type",
            "subscriptionPlan",
            "subscription_plan",
            "tier",
            "billingPlan",
            "billing_plan",
            "membership",
            "membershipType",
            "membership_type",
            "seatTier",
            "seat_tier",
            "userRateLimitTier",
            "user_rate_limit_tier",
            "organizationRateLimitTier",
            "organization_rate_limit_tier",
            "sku",
        ],
    )
    .filter(|value| {
        !matches!(
            value.trim().to_lowercase().as_str(),
            "external" | "internal" | "oauth" | "claude_ai" | "claude.ai"
        )
    })
}

fn read_claude_config_plan_type(home: &Path) -> Option<String> {
    let mut candidates = vec![home.join("settings.json")];
    if let Some(parent) = home.parent() {
        candidates.push(parent.join(".claude.json"));
    }
    if let Ok(entries) = fs::read_dir(home.join("backups")) {
        candidates.extend(entries.filter_map(Result::ok).map(|entry| entry.path()));
    }

    candidates
        .iter()
        .filter(|path| path.is_file())
        .find_map(|path| {
            let content = fs::read_to_string(path).ok()?;
            let parsed = parse_json_value(&content)?;
            find_claude_plan_type(&parsed)
        })
}

fn format_claude_plan_label(
    plan_type: Option<&str>,
    user_type: Option<&str>,
    is_claude_ai_auth: bool,
) -> String {
    if let Some(plan_type) = plan_type.map(str::trim).filter(|value| !value.is_empty()) {
        let normalized = plan_type
            .replace("Claude.ai", "")
            .replace("claude.ai", "")
            .replace("Anthropic", "")
            .replace("anthropic", "")
            .replace("Claude", "")
            .replace("claude", "")
            .replace("default", "");
        return format_plan_label("Claude", Some(&normalized));
    }

    if is_claude_ai_auth {
        return "Claude.ai account".to_string();
    }

    match user_type.unwrap_or("").trim() {
        "" => "Claude Code".to_string(),
        value => format!("Claude Code {}", value),
    }
}

fn read_claude_account(home: &Path) -> Account {
    let telemetry_path = home.join("telemetry");
    let mut latest_identity: Option<(i64, String, Option<String>, bool)> = None;
    let mut latest_plan_type: Option<(i64, String)> = None;

    if telemetry_path.exists() {
        for entry in WalkDir::new(&telemetry_path)
            .max_depth(1)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
        {
            let Ok(file) = File::open(entry.path()) else {
                continue;
            };

            for line in BufReader::new(file).lines().map_while(Result::ok) {
                let Ok(event) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let data = event.get("event_data").unwrap_or(&event);
                let email = data
                    .get("email")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let user_type = data
                    .get("user_type")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let is_claude_ai_auth = data
                    .pointer("/env/is_claude_ai_auth")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let plan_type = find_claude_plan_type(data);

                let timestamp_ms = data
                    .get("client_timestamp")
                    .and_then(Value::as_str)
                    .and_then(|timestamp| DateTime::parse_from_rfc3339(timestamp).ok())
                    .map(|timestamp| timestamp.timestamp_millis())
                    .unwrap_or(0);
                if email.is_some() || user_type.is_some() || is_claude_ai_auth {
                    let display_name = email.unwrap_or_else(|| "Claude Code".to_string());
                    let should_update = latest_identity
                        .as_ref()
                        .map(|(previous_ms, _, _, _)| timestamp_ms >= *previous_ms)
                        .unwrap_or(true);

                    if should_update {
                        latest_identity =
                            Some((timestamp_ms, display_name, user_type, is_claude_ai_auth));
                    }
                }

                if let Some(plan_type) = plan_type {
                    let should_update = latest_plan_type
                        .as_ref()
                        .map(|(previous_ms, _)| timestamp_ms >= *previous_ms)
                        .unwrap_or(true);

                    if should_update {
                        latest_plan_type = Some((timestamp_ms, plan_type));
                    }
                }
            }
        }
    }

    let (_, display_name, user_type, is_claude_ai_auth) =
        latest_identity.unwrap_or_else(|| (0, "Claude Code".to_string(), None, false));
    let telemetry_plan_type = latest_plan_type.map(|(_, plan_type)| plan_type);
    let plan_type = telemetry_plan_type.or_else(|| read_claude_config_plan_type(home));
    let plan_label = format_claude_plan_label(
        plan_type.as_deref(),
        user_type.as_deref(),
        is_claude_ai_auth,
    );
    let account_plan_type = plan_type.or(user_type);

    Account {
        initials: initials_from_name(&display_name, "CC"),
        display_name,
        plan_type: account_plan_type,
        plan_label,
        plan_monthly_usd: None,
    }
}

fn is_claude_session_file(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()) == Some("jsonl")
}

fn read_claude_stats(
    settings: &Settings,
    scan_cache: &ScanCacheStore,
    force: bool,
) -> Result<StatsScanResult, String> {
    let started_at = Instant::now();
    let chart_days = settings.chart_days.clamp(MIN_CHART_DAYS, MAX_CHART_DAYS);
    let home = claude_home(settings);
    let projects_path = home.join("projects");
    let paths = json!({
      "claudeHome": home.to_string_lossy(),
      "projectsPath": projects_path.to_string_lossy(),
      "settingsPath": home.join("settings.json").to_string_lossy(),
      "historyPath": home.join("history.jsonl").to_string_lossy()
    });

    if !projects_path.exists() {
        let stats = empty_stats(
            "Claude Code",
            "CC",
            chart_days,
            format!(
                "Claude Code session logs not found at {}",
                projects_path.to_string_lossy()
            ),
            paths,
        );
        return Ok(finish_scan(
            "claude",
            started_at,
            stats,
            ScanDiagnostics::empty("claude", force),
        ));
    }

    let files = scan_all_candidate_files(&projects_path, is_claude_session_file)
        .map_err(|failures| scan_enumeration_error("claude", failures))?;
    let outcome = scan_cache.scan(
        scan_request(
            "claude",
            "claude-session-v1",
            std::slice::from_ref(&projects_path),
            files,
            force,
        ),
        |path| {
            File::open(path).map_err(|error| error.to_string())?;
            Ok(read_claude_session(path).into_iter().collect())
        },
    );
    let diagnostics = outcome.diagnostics.clone();
    let threads = outcome.into_values();

    let account = read_claude_account(&home);
    let pricing = Some(Pricing {
        label: "Local token and usage-window estimate".to_string(),
        url: None,
        checked_at: "2026-06-30".to_string(),
    });

    let estimated_rate_limits = build_claude_estimated_rate_limits(&threads, Local::now());
    let mut stats = build_stats_from_threads(
        threads,
        Local::now(),
        chart_days,
        account,
        pricing,
        None,
        false,
        paths,
    );
    stats.rate_limits = estimated_rate_limits;

    Ok(finish_scan("claude", started_at, stats, diagnostics))
}

fn vscode_user_roots() -> Vec<PathBuf> {
    let names = ["Code", "Code - Insiders", "VSCodium", "Cursor", "Windsurf"];

    #[cfg(target_os = "macos")]
    let base = home_dir().join("Library").join("Application Support");

    #[cfg(target_os = "windows")]
    let base = env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join("AppData").join("Roaming"));

    #[cfg(all(unix, not(target_os = "macos")))]
    let base = env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join(".config"));

    names
        .iter()
        .map(|name| base.join(name).join("User"))
        .collect()
}

fn copilot_default_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for user_root in vscode_user_roots() {
        candidates.push(user_root.join("globalStorage").join("github.copilot-chat"));
        candidates.push(user_root.join("globalStorage").join("github.copilot"));
        candidates.push(user_root.join("workspaceStorage"));
    }
    candidates
}

fn chatgpt_default_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    #[cfg(target_os = "macos")]
    {
        let app_support = home_dir().join("Library").join("Application Support");
        candidates.push(app_support.join("com.openai.chat"));
        candidates.push(app_support.join("ChatGPT"));
        candidates.push(home_dir().join("Downloads").join("chatgpt-export"));
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home_dir().join("AppData").join("Roaming"));
        candidates.push(appdata.join("com.openai.chat"));
        candidates.push(appdata.join("ChatGPT"));
        candidates.push(home_dir().join("Downloads").join("chatgpt-export"));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let config = env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home_dir().join(".config"));
        candidates.push(config.join("com.openai.chat"));
        candidates.push(config.join("ChatGPT"));
        candidates.push(home_dir().join("Downloads").join("chatgpt-export"));
    }

    candidates
}

fn chatgpt_home(settings: &Settings) -> PathBuf {
    if !settings.chatgpt_home.trim().is_empty() {
        return PathBuf::from(&settings.chatgpt_home);
    }
    for key in ["AI_USAGE_CHATGPT_HOME", "CHATGPT_HOME"] {
        if let Ok(value) = env::var(key) {
            if !value.trim().is_empty() {
                return PathBuf::from(value);
            }
        }
    }

    let candidates = chatgpt_default_candidates();
    candidates
        .iter()
        .find(|path| path.exists())
        .cloned()
        .unwrap_or_else(|| {
            candidates
                .first()
                .cloned()
                .unwrap_or_else(|| home_dir().join(".chatgpt"))
        })
}

fn chatgpt_paths(settings: &Settings) -> (PathBuf, Vec<PathBuf>, Value) {
    let configured = !settings.chatgpt_home.trim().is_empty()
        || env::var("AI_USAGE_CHATGPT_HOME")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
        || env::var("CHATGPT_HOME")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
    let home = chatgpt_home(settings);
    let scan_roots = if configured {
        vec![home.clone()]
    } else {
        let existing = chatgpt_default_candidates()
            .into_iter()
            .filter(|path| path.exists())
            .collect::<Vec<_>>();
        if existing.is_empty() {
            vec![home.clone()]
        } else {
            existing
        }
    };

    let paths = json!({
      "chatgptHome": home.to_string_lossy(),
      "scanRoots": scan_roots
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect::<Vec<_>>()
    });

    (home, scan_roots, paths)
}

fn copilot_home(settings: &Settings) -> PathBuf {
    if !settings.copilot_home.trim().is_empty() {
        return PathBuf::from(&settings.copilot_home);
    }
    for key in ["GITHUB_COPILOT_HOME", "COPILOT_HOME"] {
        if let Ok(value) = env::var(key) {
            if !value.trim().is_empty() {
                return PathBuf::from(value);
            }
        }
    }

    let candidates = copilot_default_candidates();
    candidates
        .iter()
        .find(|path| path.exists())
        .cloned()
        .unwrap_or_else(|| {
            candidates
                .first()
                .cloned()
                .unwrap_or_else(|| home_dir().join(".copilot"))
        })
}

fn copilot_paths(settings: &Settings) -> (PathBuf, Vec<PathBuf>, Value) {
    let configured = !settings.copilot_home.trim().is_empty()
        || env::var("GITHUB_COPILOT_HOME")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
        || env::var("COPILOT_HOME")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
    let home = copilot_home(settings);
    let scan_roots = if configured {
        vec![home.clone()]
    } else {
        let existing = copilot_default_candidates()
            .into_iter()
            .filter(|path| path.exists())
            .collect::<Vec<_>>();
        if existing.is_empty() {
            vec![home.clone()]
        } else {
            existing
        }
    };

    let paths = json!({
      "copilotHome": home.to_string_lossy(),
      "scanRoots": scan_roots
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect::<Vec<_>>()
    });

    (home, scan_roots, paths)
}

fn object_string<'a>(object: &'a serde_json::Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn object_nested_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<&'a str> {
    if let Some(value) = object_string(object, keys) {
        return Some(value);
    }

    for key in ["author", "sender", "message", "request", "response"] {
        if let Some(Value::Object(nested)) = object.get(key) {
            if let Some(value) = object_string(nested, keys) {
                return Some(value);
            }
        }
    }

    None
}

fn timestamp_ms_from_value(value: &Value) -> Option<i64> {
    if let Some(number) = value.as_i64() {
        return Some(if number > 10_000_000_000 {
            number
        } else {
            number * 1000
        });
    }
    if let Some(number) = value.as_u64() {
        let number = number.min(i64::MAX as u64) as i64;
        return Some(if number > 10_000_000_000 {
            number
        } else {
            number * 1000
        });
    }
    let text = value.as_str()?.trim();
    if let Ok(number) = text.parse::<i64>() {
        return Some(if number > 10_000_000_000 {
            number
        } else {
            number * 1000
        });
    }
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|date| date.timestamp_millis())
}

fn object_timestamp_ms(object: &serde_json::Map<String, Value>) -> Option<i64> {
    [
        "timestamp",
        "time",
        "createdAt",
        "updatedAt",
        "lastUpdatedAt",
        "create_time",
        "update_time",
        "created_time",
        "updated_time",
        "created_at",
        "updated_at",
        "startTime",
        "endTime",
    ]
    .iter()
    .find_map(|key| object.get(*key).and_then(timestamp_ms_from_value))
}

fn estimate_tokens_from_text(text: &str) -> u64 {
    let chars = text
        .chars()
        .filter(|character| !character.is_whitespace())
        .count() as u64;
    chars.div_ceil(4).max(1)
}

fn copilot_text_tokens(value: &Value) -> u64 {
    match value {
        Value::String(text) => estimate_tokens_from_text(text),
        Value::Array(items) => items.iter().map(copilot_text_tokens).sum(),
        Value::Object(object) => object
            .get("value")
            .or_else(|| object.get("text"))
            .or_else(|| object.get("content"))
            .or_else(|| object.get("parts"))
            .map(copilot_text_tokens)
            .unwrap_or(0),
        _ => 0,
    }
}

fn copilot_usage_total(value: &Value) -> u64 {
    for key in [
        "total_tokens",
        "totalTokens",
        "totalTokenCount",
        "token_count",
        "tokenCount",
        "tokens",
    ] {
        if let Some(tokens) = value.get(key).and_then(Value::as_u64) {
            if tokens > 0 {
                return tokens;
            }
        }
    }

    [
        "input_tokens",
        "prompt_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
        "output_tokens",
        "completion_tokens",
        "inputTokens",
        "promptTokens",
        "outputTokens",
        "completionTokens",
    ]
    .iter()
    .filter_map(|key| value.get(*key).and_then(Value::as_u64))
    .sum()
}

fn copilot_object_usage_total(object: &serde_json::Map<String, Value>) -> u64 {
    for value in [
        object.get("usage"),
        object.get("tokenUsage"),
        object.get("usageInfo"),
        object.get("telemetry"),
    ]
    .into_iter()
    .flatten()
    {
        let tokens = copilot_usage_total(value);
        if tokens > 0 {
            return tokens;
        }
    }

    copilot_usage_total(&Value::Object(object.clone()))
}

fn copilot_object_text_tokens(object: &serde_json::Map<String, Value>) -> u64 {
    for key in [
        "content",
        "text",
        "prompt",
        "response",
        "completion",
        "markdown",
    ] {
        if let Some(tokens) = object
            .get(key)
            .map(copilot_text_tokens)
            .filter(|tokens| *tokens > 0)
        {
            return tokens;
        }
    }

    object
        .get("message")
        .and_then(Value::as_str)
        .map(estimate_tokens_from_text)
        .unwrap_or(0)
}

fn find_string_deep(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(object) => {
            if let Some(found) = object_string(object, keys) {
                return Some(found.to_string());
            }
            object
                .values()
                .find_map(|value| find_string_deep(value, keys))
        }
        Value::Array(items) => items.iter().find_map(|value| find_string_deep(value, keys)),
        _ => None,
    }
}

fn find_timestamp_deep(value: &Value) -> Option<i64> {
    match value {
        Value::Object(object) => {
            object_timestamp_ms(object).or_else(|| object.values().find_map(find_timestamp_deep))
        }
        Value::Array(items) => items.iter().find_map(find_timestamp_deep),
        _ => None,
    }
}

fn collect_copilot_usage_events(
    value: &Value,
    thread_id: &str,
    fallback_model: &str,
    events: &mut Vec<UsageEvent>,
    timestamps: &mut Vec<i64>,
) {
    match value {
        Value::Object(object) => {
            if let Some(timestamp_ms) = object_timestamp_ms(object) {
                timestamps.push(timestamp_ms);
            }

            let role = object_nested_string(object, &["role", "kind", "type", "speaker", "name"]);
            let role_is_message = role
                .map(|role| {
                    let role = role.to_lowercase();
                    [
                        "assistant",
                        "user",
                        "system",
                        "copilot",
                        "chatgpt",
                        "response",
                        "request",
                    ]
                    .contains(&role.as_str())
                })
                .unwrap_or(false);
            let has_message_text = copilot_object_text_tokens(object) > 0;
            let explicit_tokens = copilot_object_usage_total(object);
            let text_tokens = if explicit_tokens == 0 && (role_is_message || has_message_text) {
                copilot_object_text_tokens(object)
            } else {
                0
            };
            let total_tokens = if role_is_message || has_message_text {
                explicit_tokens.max(text_tokens)
            } else {
                0
            };

            if total_tokens > 0 {
                let timestamp_ms = object_timestamp_ms(object)
                    .or_else(|| find_timestamp_deep(value))
                    .unwrap_or(0);
                let model = object_nested_string(
                    object,
                    &[
                        "model",
                        "modelId",
                        "model_id",
                        "modelSlug",
                        "model_slug",
                        "engine",
                        "modelName",
                    ],
                )
                .unwrap_or(fallback_model)
                .to_string();
                events.push(UsageEvent {
                    thread_id: thread_id.to_string(),
                    timestamp_ms,
                    model,
                    total_tokens,
                    plan_type: None,
                    rate_limits: None,
                });
            }

            for child in object.values() {
                collect_copilot_usage_events(child, thread_id, fallback_model, events, timestamps);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_copilot_usage_events(item, thread_id, fallback_model, events, timestamps);
            }
        }
        _ => {}
    }
}

fn parse_json_from_line(line: &str) -> Option<Value> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(value);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if start >= end {
        return None;
    }
    serde_json::from_str::<Value>(&trimmed[start..=end]).ok()
}

fn read_copilot_values_as_thread(
    path: &Path,
    values: Vec<Value>,
    id_suffix: Option<&str>,
    provider_name: &str,
    default_source: &str,
    workspace_source: &str,
) -> Option<Thread> {
    let metadata = fs::metadata(path).ok();
    let fallback_id = path.file_stem()?.to_string_lossy().to_string();
    let id = values
        .iter()
        .find_map(|value| {
            find_string_deep(
                value,
                &["sessionId", "conversationId", "chatId", "threadId", "id"],
            )
        })
        .unwrap_or_else(|| match id_suffix {
            Some(suffix) => format!("{fallback_id}:{suffix}"),
            None => fallback_id.clone(),
        });
    let title = values
        .iter()
        .find_map(|value| {
            find_string_deep(value, &["title", "customTitle", "name", "summary", "label"])
        })
        .unwrap_or_else(|| format!("Copilot session {}", id.chars().take(8).collect::<String>()));
    let cwd = values
        .iter()
        .find_map(|value| {
            find_string_deep(
                value,
                &[
                    "cwd",
                    "workspaceFolder",
                    "workspace",
                    "workspacePath",
                    "rootPath",
                ],
            )
        })
        .unwrap_or_default();
    let fallback_model = values
        .iter()
        .find_map(|value| {
            find_string_deep(
                value,
                &[
                    "model",
                    "modelId",
                    "model_id",
                    "modelSlug",
                    "model_slug",
                    "engine",
                    "modelName",
                ],
            )
        })
        .unwrap_or_else(|| provider_name.to_string());

    let mut events = Vec::new();
    let mut timestamps = Vec::new();
    for value in &values {
        collect_copilot_usage_events(value, &id, &fallback_model, &mut events, &mut timestamps);
    }
    if events.is_empty() {
        return None;
    }

    let mut model_totals: HashMap<String, u64> = HashMap::new();
    for event in &events {
        *model_totals.entry(event.model.clone()).or_default() += event.total_tokens;
        if event.timestamp_ms > 0 {
            timestamps.push(event.timestamp_ms);
        }
    }
    let model = model_totals
        .into_iter()
        .max_by_key(|(_, tokens)| *tokens)
        .map(|(model, _)| model)
        .unwrap_or(fallback_model);
    let tokens_used = events.iter().map(|event| event.total_tokens).sum();
    let fallback_ms = metadata
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0);
    let created_at_ms = timestamps
        .iter()
        .copied()
        .filter(|time| *time > 0)
        .min()
        .unwrap_or(fallback_ms);
    let updated_at_ms = timestamps
        .iter()
        .copied()
        .filter(|time| *time > 0)
        .max()
        .unwrap_or(fallback_ms);
    let path_text = path.to_string_lossy().to_lowercase();
    let source = if path_text.contains("workspacestorage") {
        workspace_source.to_string()
    } else {
        default_source.to_string()
    };

    Some(Thread {
        id,
        title,
        source,
        model,
        cwd,
        archived: false,
        tokens_used,
        created_at_ms,
        updated_at_ms,
        rollout_path: path.to_string_lossy().to_string(),
        usage_events: events,
    })
}

fn read_copilot_jsonish_file(path: &Path) -> Option<Thread> {
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > 20 * 1024 * 1024 {
        return None;
    }
    let content = fs::read_to_string(path).ok()?;
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    if extension.eq_ignore_ascii_case("json") {
        let value = serde_json::from_str::<Value>(&content).ok()?;
        return read_copilot_values_as_thread(
            path,
            vec![value],
            None,
            "GitHub Copilot",
            "VS Code",
            "VS Code Workspace",
        );
    }

    let values = content
        .lines()
        .filter_map(parse_json_from_line)
        .collect::<Vec<_>>();
    if values.is_empty() {
        None
    } else {
        read_copilot_values_as_thread(
            path,
            values,
            None,
            "GitHub Copilot",
            "VS Code",
            "VS Code Workspace",
        )
    }
}

fn try_read_copilot_sqlite_threads(path: &Path) -> Result<Vec<Thread>, String> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare("select key, cast(value as text) from ItemTable")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;

    let mut threads = Vec::new();
    for row in rows {
        let (key, value) = row.map_err(|error| error.to_string())?;
        let searchable = format!("{} {}", key, value).to_lowercase();
        if !searchable.contains("copilot") && !searchable.contains("chat") {
            continue;
        }
        let Some(parsed) = serde_json::from_str::<Value>(&value).ok() else {
            continue;
        };
        if let Some(thread) = read_copilot_values_as_thread(
            path,
            vec![parsed],
            Some(&key),
            "GitHub Copilot",
            "VS Code",
            "VS Code Workspace",
        ) {
            threads.push(thread);
        }
    }
    Ok(threads)
}

fn read_chatgpt_values_as_thread(
    path: &Path,
    values: Vec<Value>,
    id_suffix: Option<&str>,
) -> Option<Thread> {
    read_copilot_values_as_thread(
        path,
        values,
        id_suffix,
        "ChatGPT",
        "ChatGPT",
        "ChatGPT Export",
    )
}

fn read_chatgpt_jsonish_file(path: &Path) -> Vec<Thread> {
    let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
        return Vec::new();
    };
    let metadata = match fs::metadata(path) {
        Ok(metadata) if metadata.len() <= 80 * 1024 * 1024 => metadata,
        _ => return Vec::new(),
    };
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };

    if extension.eq_ignore_ascii_case("json") {
        let Some(value) = serde_json::from_str::<Value>(&content).ok() else {
            return Vec::new();
        };
        if let Value::Array(items) = &value {
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("")
                .to_lowercase();
            if file_name.contains("conversation") || file_name.contains("chat") {
                return items
                    .iter()
                    .enumerate()
                    .filter_map(|(index, item)| {
                        read_chatgpt_values_as_thread(
                            path,
                            vec![item.clone()],
                            Some(&index.to_string()),
                        )
                    })
                    .collect();
            }
        }
        return read_chatgpt_values_as_thread(path, vec![value], None)
            .into_iter()
            .collect();
    }

    let values = content
        .lines()
        .filter_map(parse_json_from_line)
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Vec::new();
    }
    read_chatgpt_values_as_thread(path, values, None)
        .into_iter()
        .map(|mut thread| {
            if thread.created_at_ms == 0 {
                thread.created_at_ms = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as i64)
                    .unwrap_or(0);
            }
            thread
        })
        .collect()
}

fn system_time_to_ms(time: std::time::SystemTime) -> Option<i64> {
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as i64)
}

fn read_chatgpt_data_file(path: &Path) -> Option<Thread> {
    let path_text = path.to_string_lossy().to_lowercase();
    if !path_text.contains("conversations-v") {
        return None;
    }

    let metadata = fs::metadata(path).ok()?;
    if metadata.len() == 0 {
        return None;
    }

    let id = path.file_stem()?.to_string_lossy().to_string();
    let updated_at_ms = metadata.modified().ok().and_then(system_time_to_ms)?;
    let created_at_ms = metadata
        .created()
        .ok()
        .and_then(system_time_to_ms)
        .unwrap_or(updated_at_ms);
    let tokens_used = metadata.len().div_ceil(4).max(1);
    let model = "ChatGPT".to_string();
    let event = UsageEvent {
        thread_id: id.clone(),
        timestamp_ms: updated_at_ms,
        model: model.clone(),
        total_tokens: tokens_used,
        plan_type: None,
        rate_limits: None,
    };

    Some(Thread {
        id: id.clone(),
        title: format!(
            "ChatGPT conversation {}",
            id.chars().take(8).collect::<String>()
        ),
        source: "ChatGPT Desktop".to_string(),
        model,
        cwd: String::new(),
        archived: false,
        tokens_used,
        created_at_ms,
        updated_at_ms,
        rollout_path: path.to_string_lossy().to_string(),
        usage_events: vec![event],
    })
}

fn try_read_chatgpt_sqlite_threads(path: &Path) -> Result<Vec<Thread>, String> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| error.to_string())?;
    let mut tables = connection
        .prepare("select name from sqlite_master where type = 'table' and name not like 'sqlite_%'")
        .map_err(|error| error.to_string())?;
    let table_rows = tables
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;

    let mut threads = Vec::new();
    let mut scanned_rows = 0_usize;
    for table in table_rows {
        let table = table.map_err(|error| error.to_string())?;
        if scanned_rows >= CHATGPT_SQLITE_ROW_SCAN_LIMIT {
            break;
        }
        if !table
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            continue;
        }
        let remaining_rows = CHATGPT_SQLITE_ROW_SCAN_LIMIT - scanned_rows;
        let mut statement = connection
            .prepare(&format!("select * from \"{table}\" limit {remaining_rows}"))
            .map_err(|error| error.to_string())?;
        let column_count = statement.column_count();
        let column_names = statement
            .column_names()
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>();
        let rows = statement
            .query_map([], |row| {
                let mut object = serde_json::Map::new();
                for index in 0..column_count {
                    let name = column_names
                        .get(index)
                        .cloned()
                        .unwrap_or_else(|| format!("column{index}"));
                    if let Ok(value) = row.get::<_, String>(index) {
                        object.insert(name, Value::String(value));
                    } else if let Ok(value) = row.get::<_, i64>(index) {
                        object.insert(name, Value::Number(value.into()));
                    }
                }
                Ok(Value::Object(object))
            })
            .map_err(|error| error.to_string())?;

        for (index, value) in rows.enumerate() {
            let value = value.map_err(|error| error.to_string())?;
            scanned_rows += 1;
            let searchable = value.to_string().to_lowercase();
            if !["chatgpt", "openai", "conversation", "message", "gpt"]
                .iter()
                .any(|needle| searchable.contains(needle))
            {
                continue;
            }
            let parsed_values = if let Some(parsed) = value
                .as_object()
                .into_iter()
                .flat_map(|object| object.values())
                .filter_map(Value::as_str)
                .find_map(parse_json_value)
            {
                vec![parsed]
            } else {
                vec![value]
            };
            if let Some(thread) = read_chatgpt_values_as_thread(
                path,
                parsed_values,
                Some(&format!("{table}:{index}")),
            ) {
                threads.push(thread);
            }

            if scanned_rows >= CHATGPT_SQLITE_ROW_SCAN_LIMIT {
                break;
            }
        }
    }

    Ok(threads)
}

fn build_chatgpt_activity_summary(
    threads: &[Thread],
    now: DateTime<Local>,
) -> Option<ActivitySummary> {
    if threads.is_empty() {
        return None;
    }

    let now_ms = now.timestamp_millis();
    let activity_times = threads
        .iter()
        .flat_map(|thread| {
            let event_times = thread
                .usage_events
                .iter()
                .filter(|event| event.timestamp_ms > 0)
                .map(|event| event.timestamp_ms)
                .collect::<Vec<_>>();
            if event_times.is_empty() {
                vec![thread.updated_at_ms]
            } else {
                event_times
            }
        })
        .filter(|timestamp_ms| *timestamp_ms > 0 && *timestamp_ms <= now_ms)
        .collect::<Vec<_>>();

    if activity_times.is_empty() {
        return None;
    }

    let windows = [("recent", 180_u64), ("weekly", 10080_u64)]
        .into_iter()
        .map(|(id, window_minutes)| {
            let window_start_ms = now_ms - window_minutes as i64 * 60 * 1000;
            let count = activity_times
                .iter()
                .filter(|timestamp_ms| **timestamp_ms >= window_start_ms)
                .count() as u64;

            ActivityWindow {
                id: id.to_string(),
                label: rate_limit_window_label(window_minutes),
                count,
                window_minutes,
            }
        })
        .collect();

    Some(ActivitySummary {
        updated_at: Some(iso_now()),
        windows,
    })
}

fn state_db_candidates(scan_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    for root in scan_roots {
        if root.file_name().and_then(|name| name.to_str()) == Some("state.vscdb") {
            candidates.push(root.clone());
        }
        candidates.push(root.join("state.vscdb"));
        if let Some(parent) = root.parent() {
            candidates.push(parent.join("state.vscdb"));
        }
    }

    candidates.sort();
    candidates.dedup();
    candidates
}

fn storage_json_candidates(scan_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    for root in scan_roots {
        candidates.push(root.join("storage.json"));
        if let Some(parent) = root.parent() {
            candidates.push(parent.join("storage.json"));
        }
    }

    candidates.sort();
    candidates.dedup();
    candidates
}

fn read_state_items(path: &Path) -> Vec<(String, String)> {
    let Ok(connection) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return Vec::new();
    };
    let Ok(mut statement) = connection.prepare("select key, cast(value as text) from ItemTable")
    else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return Vec::new();
    };

    rows.filter_map(Result::ok).collect()
}

fn find_string_deep_ci(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if keys
                    .iter()
                    .any(|candidate| key.eq_ignore_ascii_case(candidate))
                {
                    if let Some(text) = value.as_str() {
                        let text = text.trim();
                        if is_display_identifier(text) {
                            return Some(text.to_string());
                        }
                    }
                }
            }
            object
                .values()
                .find_map(|value| find_string_deep_ci(value, keys))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|value| find_string_deep_ci(value, keys)),
        _ => None,
    }
}

fn is_display_identifier(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 120
        && !value.contains('\n')
        && !value.to_lowercase().contains("token")
        && !value.starts_with("eyJ")
}

fn format_plan_label(provider_name: &str, raw: Option<&str>) -> String {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return provider_name.to_string();
    };

    let normalized = raw
        .replace(provider_name, "")
        .replace("github", "")
        .replace("copilot", "")
        .replace(['_', '-', '.'], " ");
    let words = normalized
        .split_whitespace()
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();

    if words.is_empty() {
        provider_name.to_string()
    } else {
        format!("{provider_name} {}", words.join(" "))
    }
}

fn parse_json_value(value: &str) -> Option<Value> {
    serde_json::from_str::<Value>(value).ok()
}

fn parse_json_or_nested_value(value: &str) -> Option<Value> {
    let parsed = parse_json_value(value)?;
    if let Value::String(text) = &parsed {
        let trimmed = text.trim();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            if let Some(nested) = parse_json_value(trimmed) {
                return Some(nested);
            }
        }
    }
    Some(parsed)
}

fn plain_display_value(value: &str) -> Option<String> {
    match parse_json_value(value) {
        Some(Value::String(text)) => {
            let text = text.trim();
            is_display_identifier(text).then(|| text.to_string())
        }
        _ => {
            let text = value.trim();
            is_display_identifier(text).then(|| text.to_string())
        }
    }
}

fn key_contains_ci(key_lower: &str, candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|candidate| key_lower.contains(&candidate.to_lowercase()))
}

fn value_as_f64(value: &Value) -> Option<f64> {
    if let Some(number) = value.as_f64() {
        return Some(number);
    }
    value.as_str()?.trim().parse::<f64>().ok()
}

fn value_as_u64(value: &Value) -> Option<u64> {
    if let Some(number) = value.as_u64() {
        return Some(number);
    }
    value.as_str()?.trim().parse::<u64>().ok()
}

fn object_f64(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(value_as_f64))
}

fn object_u64(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(value_as_u64))
}

fn read_github_copilot_account(scan_roots: &[PathBuf]) -> Account {
    let mut state_dbs = state_db_candidates(scan_roots);
    if !state_dbs.iter().any(|path| path.exists()) {
        for user_root in vscode_user_roots() {
            state_dbs.push(user_root.join("globalStorage").join("state.vscdb"));
        }
    }
    state_dbs.sort();
    state_dbs.dedup();

    let mut login: Option<String> = None;
    let mut plan_type: Option<String> = None;
    let mut account_id: Option<String> = None;

    for db_path in state_dbs.iter().filter(|path| path.exists()) {
        for (key, value) in read_state_items(db_path) {
            let key_lower = key.to_lowercase();
            if login.is_none()
                && key_lower.starts_with("github-")
                && !key_lower.ends_with("-usages")
            {
                let candidate = key.trim_start_matches("github-").trim();
                if is_display_identifier(candidate) {
                    login = Some(candidate.to_string());
                }
            }

            if (key == "extensionsAssignmentFilterProvider.copilotSku"
                || key == "exp.github.copilot.sku")
                && is_display_identifier(&value)
            {
                plan_type = Some(value.clone());
            }

            if let Some(parsed) = parse_json_value(&value) {
                if plan_type.is_none() {
                    plan_type = find_string_deep_ci(
                        &parsed,
                        &[
                            "exp.github.copilot.sku",
                            "copilotSku",
                            "sku",
                            "plan",
                            "planType",
                        ],
                    );
                }
                if account_id.is_none() {
                    account_id = find_string_deep_ci(&parsed, &["accountId", "account_id"]);
                }
            }
        }
    }

    let display_name = login
        .or_else(|| account_id.map(|id| format!("GitHub {id}")))
        .unwrap_or_else(|| "GitHub Copilot".to_string());
    let plan_label = format_plan_label("GitHub Copilot", plan_type.as_deref());

    Account {
        initials: initials_from_name(&display_name, "GH"),
        display_name,
        plan_type,
        plan_label,
        plan_monthly_usd: None,
    }
}

fn read_cursor_account(scan_roots: &[PathBuf]) -> Account {
    let display_keys = [
        "email",
        "login",
        "username",
        "userName",
        "displayName",
        "name",
    ];
    let plan_keys = [
        "plan",
        "planType",
        "tier",
        "membershipType",
        "subscription",
        "subscriptionType",
        "sku",
    ];
    let mut display_name: Option<String> = None;
    let mut plan_type: Option<String> = None;

    for db_path in state_db_candidates(scan_roots)
        .iter()
        .filter(|path| path.exists())
    {
        for (key, value) in read_state_items(db_path) {
            let key_lower = key.to_lowercase();
            if key_lower.contains("token") || key_lower.contains("secret") {
                continue;
            }
            if ![
                "cursor",
                "account",
                "profile",
                "user",
                "auth",
                "membership",
                "subscription",
            ]
            .iter()
            .any(|needle| key_lower.contains(needle))
            {
                continue;
            }
            let parsed = parse_json_or_nested_value(&value);
            if display_name.is_none() && key_contains_ci(&key_lower, &display_keys[..5]) {
                display_name = plain_display_value(&value);
            }
            if display_name.is_none() {
                if let Some(parsed) = parsed.as_ref() {
                    display_name = find_string_deep_ci(parsed, &display_keys);
                }
            }
            if plan_type.is_none() && key_contains_ci(&key_lower, &plan_keys) {
                plan_type = plain_display_value(&value);
            }
            if plan_type.is_none() {
                if let Some(parsed) = parsed.as_ref() {
                    plan_type = find_string_deep_ci(parsed, &plan_keys);
                }
            }
        }
    }

    for json_path in storage_json_candidates(scan_roots)
        .iter()
        .filter(|path| path.exists())
    {
        let Ok(content) = fs::read_to_string(json_path) else {
            continue;
        };
        let Some(parsed) = parse_json_value(&content) else {
            continue;
        };
        if display_name.is_none() {
            display_name = find_string_deep_ci(&parsed, &display_keys);
        }
        if plan_type.is_none() {
            plan_type = find_string_deep_ci(&parsed, &plan_keys);
        }
    }

    let display_name = display_name.unwrap_or_else(|| "Cursor".to_string());
    let plan_label = format_plan_label("Cursor", plan_type.as_deref());

    Account {
        initials: initials_from_name(&display_name, "CU"),
        display_name,
        plan_type,
        plan_label,
        plan_monthly_usd: None,
    }
}

fn chatgpt_user_label_from_path(path: &Path) -> Option<String> {
    for component in path.components() {
        let text = component.as_os_str().to_string_lossy();
        let Some(start) = text.find("user-") else {
            continue;
        };
        let suffix = &text[start + "user-".len()..];
        let user_id =
            suffix
                .split("__")
                .next()
                .unwrap_or(suffix)
                .trim_matches(|character: char| {
                    !(character.is_ascii_alphanumeric() || character == '-' || character == '_')
                });
        if user_id.is_empty() {
            continue;
        }
        let short_id = user_id.chars().take(8).collect::<String>();
        if is_display_identifier(&short_id) {
            return Some(format!("ChatGPT user {short_id}"));
        }
    }

    None
}

fn read_chatgpt_account(scan_roots: &[PathBuf]) -> Account {
    let display_keys = [
        "email",
        "login",
        "username",
        "userName",
        "displayName",
        "name",
    ];
    let plan_keys = [
        "plan",
        "planType",
        "tier",
        "subscription",
        "subscriptionType",
        "sku",
    ];
    let mut display_name: Option<String> = None;
    let mut plan_type: Option<String> = None;
    let mut local_user_label: Option<String> = None;

    for json_path in scan_roots.iter().filter(|path| path.exists()) {
        if local_user_label.is_none() {
            local_user_label = chatgpt_user_label_from_path(json_path);
        }
        if local_user_label.is_none() && json_path.is_dir() {
            local_user_label = WalkDir::new(json_path)
                .max_depth(4)
                .into_iter()
                .filter_map(Result::ok)
                .find_map(|entry| chatgpt_user_label_from_path(entry.path()));
        }

        let mut read_account_file = |file: &Path| {
            if local_user_label.is_none() {
                local_user_label = chatgpt_user_label_from_path(file);
            }
            let Ok(content) = fs::read_to_string(file) else {
                return;
            };
            let Some(parsed) = parse_json_value(&content) else {
                return;
            };
            if display_name.is_none() {
                display_name = find_string_deep_ci(&parsed, &display_keys);
            }
            if plan_type.is_none() {
                plan_type = find_string_deep_ci(&parsed, &plan_keys);
            }
        };

        if json_path.is_file() {
            read_account_file(json_path);
        } else {
            for entry in WalkDir::new(json_path)
                .max_depth(4)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_file())
            {
                let file = entry.path();
                let name = file.to_string_lossy().to_lowercase();
                if name.ends_with(".json")
                    && ["account", "profile", "user", "settings", "session", "auth"]
                        .iter()
                        .any(|needle| name.contains(needle))
                {
                    read_account_file(file);
                }
            }
        }
    }

    let display_name = display_name
        .or(local_user_label)
        .unwrap_or_else(|| "ChatGPT".to_string());
    let plan_label = format_plan_label("ChatGPT", plan_type.as_deref());
    let plan_monthly_usd = plan_monthly_usd(plan_type.as_deref());

    Account {
        initials: if display_name.starts_with("ChatGPT user ") {
            "CG".to_string()
        } else {
            initials_from_name(&display_name, "CG")
        },
        display_name,
        plan_type,
        plan_label,
        plan_monthly_usd,
    }
}

fn cursor_user_roots() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    let base = home_dir().join("Library").join("Application Support");

    #[cfg(target_os = "windows")]
    let base = env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join("AppData").join("Roaming"));

    #[cfg(all(unix, not(target_os = "macos")))]
    let base = env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join(".config"));

    vec![base.join("Cursor").join("User")]
}

fn cursor_default_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for user_root in cursor_user_roots() {
        candidates.push(user_root.join("globalStorage"));
        candidates.push(user_root.join("workspaceStorage"));
    }
    candidates
}

fn cursor_scan_roots_for_home(home: &Path) -> Vec<PathBuf> {
    let mut roots = vec![home.to_path_buf()];
    if home.is_file() {
        return roots;
    }

    let file_name = home.file_name().and_then(|name| name.to_str());
    if matches!(file_name, Some("globalStorage" | "workspaceStorage")) {
        if let Some(user_root) = home.parent() {
            roots.push(user_root.join("globalStorage"));
            roots.push(user_root.join("workspaceStorage"));
        }
    } else if file_name == Some("User") {
        roots.push(home.join("globalStorage"));
        roots.push(home.join("workspaceStorage"));
    } else if let Some(parent) = home.parent() {
        if parent.file_name().and_then(|name| name.to_str()) == Some("workspaceStorage") {
            roots.push(parent.to_path_buf());
            if let Some(user_root) = parent.parent() {
                roots.push(user_root.join("globalStorage"));
            }
        }
    }

    roots.sort();
    roots.dedup();
    roots
}

fn cursor_home(settings: &Settings) -> PathBuf {
    if !settings.cursor_home.trim().is_empty() {
        return PathBuf::from(&settings.cursor_home);
    }
    for key in ["AI_USAGE_CURSOR_HOME", "CURSOR_HOME"] {
        if let Ok(value) = env::var(key) {
            if !value.trim().is_empty() {
                return PathBuf::from(value);
            }
        }
    }

    let candidates = cursor_default_candidates();
    candidates
        .iter()
        .find(|path| path.exists())
        .cloned()
        .unwrap_or_else(|| {
            candidates
                .first()
                .cloned()
                .unwrap_or_else(|| home_dir().join(".cursor"))
        })
}

fn cursor_paths(settings: &Settings) -> (PathBuf, Vec<PathBuf>, Value) {
    let configured = !settings.cursor_home.trim().is_empty()
        || env::var("AI_USAGE_CURSOR_HOME")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
        || env::var("CURSOR_HOME")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
    let home = cursor_home(settings);
    let scan_roots = if configured {
        cursor_scan_roots_for_home(&home)
    } else {
        let existing = cursor_default_candidates()
            .into_iter()
            .filter(|path| path.exists())
            .collect::<Vec<_>>();
        if existing.is_empty() {
            vec![home.clone()]
        } else {
            existing
        }
    };

    let paths = json!({
      "cursorHome": home.to_string_lossy(),
      "scanRoots": scan_roots
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect::<Vec<_>>()
    });

    (home, scan_roots, paths)
}

fn read_cursor_values_as_thread(
    path: &Path,
    values: Vec<Value>,
    id_suffix: Option<&str>,
) -> Option<Thread> {
    read_copilot_values_as_thread(
        path,
        values,
        id_suffix,
        "Cursor",
        "Cursor",
        "Cursor Workspace",
    )
}

fn cursor_header_token_estimate(object: &serde_json::Map<String, Value>) -> u64 {
    let explicit = object_u64(
        object,
        &[
            "total_tokens",
            "totalTokens",
            "tokenCount",
            "tokensUsed",
            "tokens",
        ],
    )
    .unwrap_or(0);
    if explicit > 0 {
        return explicit;
    }

    if let Some(percent) = object_f64(object, &["contextUsagePercent"]) {
        if percent > 0.0 {
            return (percent * 100.0).round().max(1.0) as u64;
        }
    }

    let changed_lines = object_u64(object, &["totalLinesAdded"]).unwrap_or(0)
        + object_u64(object, &["totalLinesRemoved"]).unwrap_or(0);
    changed_lines.saturating_mul(12)
}

fn cursor_workspace_from_header(object: &serde_json::Map<String, Value>) -> String {
    object
        .get("workspaceIdentifier")
        .and_then(|value| find_string_deep(value, &["fsPath", "workspacePath", "rootPath"]))
        .or_else(|| {
            object
                .get("workspaceIdentifier")
                .and_then(|value| find_string_deep(value, &["path"]))
                .filter(|path| path.starts_with('/'))
        })
        .unwrap_or_default()
}

fn read_cursor_composer_headers(path: &Path, value: &Value) -> Vec<Thread> {
    let headers = value
        .get("allComposers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let mut threads = Vec::new();

    for header in headers {
        let Some(object) = header.as_object() else {
            continue;
        };
        if object
            .get("isDraft")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let Some(id) = object_string(object, &["composerId", "id"]).map(str::to_string) else {
            continue;
        };
        let created_at_ms = object
            .get("createdAt")
            .and_then(timestamp_ms_from_value)
            .or_else(|| {
                object
                    .get("lastUpdatedAt")
                    .and_then(timestamp_ms_from_value)
            })
            .unwrap_or(0);
        let updated_at_ms = object
            .get("lastUpdatedAt")
            .and_then(timestamp_ms_from_value)
            .or_else(|| {
                object
                    .get("conversationCheckpointLastUpdatedAt")
                    .and_then(timestamp_ms_from_value)
            })
            .unwrap_or(created_at_ms);
        let tokens_used = cursor_header_token_estimate(object);
        let model = object_string(object, &["unifiedMode", "forceMode"])
            .map(|mode| format!("Cursor {mode}"))
            .unwrap_or_else(|| "Cursor".to_string());
        let event = UsageEvent {
            thread_id: id.clone(),
            timestamp_ms: updated_at_ms,
            model: model.clone(),
            total_tokens: tokens_used,
            plan_type: None,
            rate_limits: None,
        };

        threads.push(Thread {
            id: id.clone(),
            title: object_string(object, &["name", "title"])
                .map(str::to_string)
                .unwrap_or_else(|| {
                    format!("Cursor session {}", id.chars().take(8).collect::<String>())
                }),
            source: "Cursor Composer".to_string(),
            model,
            cwd: cursor_workspace_from_header(object),
            archived: object
                .get("isArchived")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            tokens_used,
            created_at_ms,
            updated_at_ms,
            rollout_path: path.to_string_lossy().to_string(),
            usage_events: if tokens_used > 0 {
                vec![event]
            } else {
                Vec::new()
            },
        });
    }

    threads
}

fn read_cursor_jsonish_file(path: &Path) -> Option<Thread> {
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > 20 * 1024 * 1024 {
        return None;
    }
    let content = fs::read_to_string(path).ok()?;
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    if extension.eq_ignore_ascii_case("json") {
        let value = serde_json::from_str::<Value>(&content).ok()?;
        return read_cursor_values_as_thread(path, vec![value], None);
    }

    let values = content
        .lines()
        .filter_map(parse_json_from_line)
        .collect::<Vec<_>>();
    if values.is_empty() {
        None
    } else {
        read_cursor_values_as_thread(path, values, None)
    }
}

#[cfg(test)]
fn read_cursor_sqlite_threads(path: &Path) -> Vec<Thread> {
    try_read_cursor_sqlite_threads(path).unwrap_or_default()
}

fn try_read_cursor_sqlite_threads(path: &Path) -> Result<Vec<Thread>, String> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| error.to_string())?;
    let mut statement = connection
        .prepare("select key, cast(value as text) from ItemTable")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;

    let mut threads = Vec::new();
    for row in rows {
        let (key, value) = row.map_err(|error| error.to_string())?;
        let searchable = format!("{} {}", key, value).to_lowercase();
        if ![
            "cursor",
            "chat",
            "composer",
            "aichat",
            "ai_chat",
            "workbench.panel.aichat",
        ]
        .iter()
        .any(|needle| searchable.contains(needle))
        {
            continue;
        }
        let Some(parsed) = parse_json_or_nested_value(&value) else {
            continue;
        };
        if key == "composer.composerHeaders" {
            threads.extend(read_cursor_composer_headers(path, &parsed));
        } else if let Some(thread) = read_cursor_values_as_thread(path, vec![parsed], Some(&key)) {
            threads.push(thread);
        }
    }

    Ok(threads)
}

fn scan_candidate_files(
    root: &Path,
    is_candidate: fn(&Path) -> bool,
) -> Result<Vec<PathBuf>, usize> {
    scan_candidate_files_with_limits(
        root,
        is_candidate,
        Some(DEFAULT_SCAN_MAX_DEPTH),
        Some(DEFAULT_SCAN_MAX_FILES),
    )
}

fn scan_all_candidate_files(
    root: &Path,
    is_candidate: fn(&Path) -> bool,
) -> Result<Vec<PathBuf>, usize> {
    scan_candidate_files_with_limits(root, is_candidate, None, None)
}

fn scan_candidate_files_with_limits(
    root: &Path,
    is_candidate: fn(&Path) -> bool,
    max_depth: Option<usize>,
    max_files: Option<usize>,
) -> Result<Vec<PathBuf>, usize> {
    if root.is_file() {
        return Ok(is_candidate(root)
            .then(|| root.to_path_buf())
            .into_iter()
            .collect());
    }

    let mut visited_files = 0_usize;
    let mut files = Vec::new();
    let mut failures = 0_usize;
    let walker = max_depth
        .map(|max_depth| WalkDir::new(root).max_depth(max_depth))
        .unwrap_or_else(|| WalkDir::new(root));
    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                failures += 1;
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }

        visited_files += 1;
        if max_files.is_some_and(|max_files| visited_files > max_files) {
            failures += 1;
            break;
        }

        let path = entry.path();
        if is_candidate(path) {
            files.push(path.to_path_buf());
        }
    }

    if failures == 0 {
        Ok(files)
    } else {
        Err(failures)
    }
}

fn scan_enumeration_error(provider: &str, failures: usize) -> String {
    eprintln!("[scan] provider={provider} enumeration_incomplete=true failed_entries={failures}");
    format!("Unable to completely enumerate {provider} data files ({failures} failures)")
}

fn is_cursor_data_file(path: &Path) -> bool {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_lowercase();
    if file_name == "state.vscdb" {
        return true;
    }

    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !["json", "jsonl", "log"].contains(&extension.as_str()) {
        return false;
    }

    let path_text = path.to_string_lossy().to_lowercase();
    ["cursor", "chat", "composer", "aichat", "ai_chat"]
        .iter()
        .any(|needle| path_text.contains(needle))
}

fn is_chatgpt_data_file(path: &Path) -> bool {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_lowercase();
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();

    if ["sqlite", "sqlite3", "db"].contains(&extension.as_str()) {
        let path_text = path.to_string_lossy().to_lowercase();
        return [
            "chatgpt",
            "openai",
            "conversation",
            "message",
            "com.openai.chat",
        ]
        .iter()
        .any(|needle| path_text.contains(needle));
    }
    if extension == "data" {
        return path
            .to_string_lossy()
            .to_lowercase()
            .contains("conversations-v");
    }
    if !["json", "jsonl", "log"].contains(&extension.as_str()) {
        return false;
    }
    if file_name == "conversations.json" || file_name == "chat.json" {
        return true;
    }

    let path_text = path.to_string_lossy().to_lowercase();
    ["chatgpt", "openai", "conversation", "message"]
        .iter()
        .any(|needle| path_text.contains(needle))
}

fn is_copilot_data_file(path: &Path) -> bool {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_lowercase();
    if file_name == "state.vscdb" {
        return true;
    }
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !["json", "jsonl", "log"].contains(&extension.as_str()) {
        return false;
    }
    let path_text = path.to_string_lossy().to_lowercase();
    path_text.contains("copilot") || path_text.contains("chat")
}

fn read_chatgpt_threads_from_path(path: &Path) -> Result<Vec<Thread>, String> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ["sqlite", "sqlite3", "db"].contains(&extension.as_str()) {
        try_read_chatgpt_sqlite_threads(path)
    } else if extension == "data" {
        Ok(read_chatgpt_data_file(path).into_iter().collect())
    } else {
        Ok(read_chatgpt_jsonish_file(path))
    }
}

fn read_chatgpt_stats(
    settings: &Settings,
    scan_cache: &ScanCacheStore,
    force: bool,
) -> Result<StatsScanResult, String> {
    let started_at = Instant::now();
    let chart_days = settings.chart_days.clamp(MIN_CHART_DAYS, MAX_CHART_DAYS);
    let (home, scan_roots, paths) = chatgpt_paths(settings);

    if !scan_roots.iter().any(|path| path.exists()) {
        let stats = empty_stats(
            "ChatGPT",
            "CG",
            chart_days,
            format!("ChatGPT data not found at {}", home.to_string_lossy()),
            paths,
        );
        return Ok(finish_scan(
            "chatgpt",
            started_at,
            stats,
            ScanDiagnostics::empty("chatgpt", force),
        ));
    }

    let mut files = Vec::new();
    for root in scan_roots.iter().filter(|path| path.exists()) {
        files.extend(
            scan_candidate_files(root, is_chatgpt_data_file)
                .map_err(|failures| scan_enumeration_error("chatgpt", failures))?,
        );
    }
    let outcome = scan_cache.scan(
        scan_request("chatgpt", "chatgpt-data-v1", &scan_roots, files, force),
        |path| {
            File::open(path).map_err(|error| error.to_string())?;
            read_chatgpt_threads_from_path(path)
        },
    );
    let diagnostics = outcome.diagnostics.clone();
    let threads = outcome.into_values();

    let account = read_chatgpt_account(&scan_roots);
    let pricing = Some(Pricing {
        label: "ChatGPT local activity estimate".to_string(),
        url: None,
        checked_at: "2026-07-02".to_string(),
    });

    let activity = build_chatgpt_activity_summary(&threads, Local::now());
    let mut stats = build_stats_from_threads(
        threads,
        Local::now(),
        chart_days,
        account,
        pricing,
        None,
        false,
        paths,
    );
    stats.activity = activity;

    Ok(finish_scan("chatgpt", started_at, stats, diagnostics))
}

fn read_copilot_stats(
    settings: &Settings,
    scan_cache: &ScanCacheStore,
    force: bool,
) -> Result<StatsScanResult, String> {
    let started_at = Instant::now();
    let chart_days = settings.chart_days.clamp(MIN_CHART_DAYS, MAX_CHART_DAYS);
    let (home, scan_roots, paths) = copilot_paths(settings);

    if !scan_roots.iter().any(|path| path.exists()) {
        let stats = empty_stats(
            "GitHub Copilot",
            "GH",
            chart_days,
            format!(
                "GitHub Copilot local storage not found at {}",
                home.to_string_lossy()
            ),
            paths,
        );
        return Ok(finish_scan(
            "copilot",
            started_at,
            stats,
            ScanDiagnostics::empty("copilot", force),
        ));
    }

    let mut files = Vec::new();
    for root in scan_roots.iter().filter(|path| path.exists()) {
        files.extend(
            scan_candidate_files(root, is_copilot_data_file)
                .map_err(|failures| scan_enumeration_error("copilot", failures))?,
        );
    }
    let outcome = scan_cache.scan(
        scan_request("copilot", "copilot-data-v1", &scan_roots, files, force),
        |path| {
            File::open(path).map_err(|error| error.to_string())?;
            if path
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default()
                .eq_ignore_ascii_case("state.vscdb")
            {
                try_read_copilot_sqlite_threads(path)
            } else if let Some(thread) = read_copilot_jsonish_file(path) {
                Ok(vec![thread])
            } else {
                Ok(Vec::new())
            }
        },
    );
    let diagnostics = outcome.diagnostics.clone();
    let threads = outcome.into_values();

    let account = read_github_copilot_account(&scan_roots);
    let pricing = Some(Pricing {
        label: "GitHub Copilot local token estimate".to_string(),
        url: None,
        checked_at: "2026-06-30".to_string(),
    });

    let stats = build_stats_from_threads(
        threads,
        Local::now(),
        chart_days,
        account,
        pricing,
        None,
        false,
        paths,
    );
    Ok(finish_scan("copilot", started_at, stats, diagnostics))
}

fn read_cursor_stats(
    settings: &Settings,
    scan_cache: &ScanCacheStore,
    force: bool,
) -> Result<StatsScanResult, String> {
    let started_at = Instant::now();
    let chart_days = settings.chart_days.clamp(MIN_CHART_DAYS, MAX_CHART_DAYS);
    let (home, scan_roots, paths) = cursor_paths(settings);

    if !scan_roots.iter().any(|path| path.exists()) {
        let stats = empty_stats(
            "Cursor",
            "CU",
            chart_days,
            format!(
                "Cursor local storage not found at {}",
                home.to_string_lossy()
            ),
            paths,
        );
        return Ok(finish_scan(
            "cursor",
            started_at,
            stats,
            ScanDiagnostics::empty("cursor", force),
        ));
    }

    let mut files = Vec::new();
    for root in scan_roots.iter().filter(|path| path.exists()) {
        files.extend(
            scan_candidate_files(root, is_cursor_data_file)
                .map_err(|failures| scan_enumeration_error("cursor", failures))?,
        );
    }
    let outcome = scan_cache.scan(
        scan_request("cursor", "cursor-data-v1", &scan_roots, files, force),
        |path| {
            File::open(path).map_err(|error| error.to_string())?;
            if path
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default()
                .eq_ignore_ascii_case("state.vscdb")
            {
                try_read_cursor_sqlite_threads(path)
            } else if let Some(thread) = read_cursor_jsonish_file(path) {
                Ok(vec![thread])
            } else {
                Ok(Vec::new())
            }
        },
    );
    let diagnostics = outcome.diagnostics.clone();
    let threads = outcome.into_values();

    let account = read_cursor_account(&scan_roots);
    let pricing = Some(Pricing {
        label: "Cursor local token estimate".to_string(),
        url: None,
        checked_at: "2026-06-30".to_string(),
    });

    let stats = build_stats_from_threads(
        threads,
        Local::now(),
        chart_days,
        account,
        pricing,
        None,
        false,
        paths,
    );
    Ok(finish_scan("cursor", started_at, stats, diagnostics))
}

fn main() {
    let builder = tauri::Builder::default();

    #[cfg(any(windows, target_os = "linux"))]
    let builder = {
        let mut builder = builder;
        if let Some(pubkey) = updater_public_key() {
            builder = builder.plugin(tauri_plugin_updater::Builder::new().pubkey(pubkey).build());
        }
        builder
    };

    let app = builder
        .setup(setup_tray)
        .on_window_event(|window, event| {
            if window.label() == TRAY_STATUS_WINDOW_LABEL {
                match event {
                    tauri::WindowEvent::Focused(false) => {
                        let _ = window.hide();
                    }
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                    _ => {}
                }
                return;
            }
            if window.label() != "main" {
                return;
            }
            #[cfg(target_os = "windows")]
            if let tauri::WindowEvent::ScaleFactorChanged { scale_factor, .. } = event {
                if let Some(webview_window) = window.app_handle().get_webview_window("main") {
                    if let Err(error) = refresh_windows_window_icons(&webview_window, *scale_factor)
                    {
                        eprintln!("failed to refresh Windows window icons: {error}");
                    }
                }
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            update_settings,
            sync_tray_language,
            get_tray_status,
            open_main_window,
            get_stats,
            rebuild_stats_cache,
            get_scan_diagnostics,
            choose_home,
            start_window_drag,
            open_external,
            check_update,
            install_update
        ])
        .build(tauri::generate_context!())
        .expect("error while building Dial");
    app.run(handle_run_event);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static TEST_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_account() -> Account {
        Account {
            display_name: "Codex".to_string(),
            initials: "CD".to_string(),
            plan_type: None,
            plan_label: "Codex".to_string(),
            plan_monthly_usd: None,
        }
    }

    fn test_rate_limit_window(id: &str, remaining_percent: f64) -> RateLimitWindow {
        RateLimitWindow {
            id: id.to_string(),
            label: id.to_string(),
            used_percent: 100.0 - remaining_percent,
            remaining_percent,
            window_minutes: 300,
            resets_at: None,
        }
    }

    fn test_tray_display() -> TrayDisplayState {
        TrayDisplayState {
            language: TrayLanguage::Zh,
            active_provider: "codex".to_string(),
            remaining_percent: Some(50),
            theme: "system".to_string(),
            accent_color: "blue".to_string(),
            auto_refresh_enabled: false,
            auto_refresh_minutes: 30,
            is_refreshing: false,
            refresh_failed: false,
            refresh_in_flight: false,
            pending_refresh_request_id: None,
            latest_refresh_request_id: 0,
        }
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn updater_timestamp_uses_browser_compatible_rfc3339() {
        let date = time::OffsetDateTime::from_unix_timestamp(0).unwrap();

        assert_eq!(
            updater_published_at(date).as_deref(),
            Some("1970-01-01T00:00:00.000Z")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn tray_template_icon_is_square_with_transparent_background() {
        let icon = tray_template_icon();
        let alpha = icon.rgba().iter().skip(3).step_by(4).copied();

        assert_eq!(icon.width(), 32);
        assert_eq!(icon.height(), 32);
        assert!(alpha.clone().any(|value| value == 0));
        assert!(alpha.clone().any(|value| value == u8::MAX));
        assert_eq!(icon.rgba()[3], 0);
        assert_eq!(icon.rgba()[(2 * 32 + 16) * 4 + 3], 0);
        assert_eq!(icon.rgba()[(7 * 32 + 16) * 4 + 3], u8::MAX);
        assert_eq!(icon.rgba()[(19 * 32 + 16) * 4 + 3], u8::MAX);
    }

    #[test]
    fn tray_status_payload_follows_selected_language() {
        let mut display = test_tray_display();
        display.remaining_percent = Some(98);

        let payload = tray_status_payload(&display);
        assert_eq!(payload.provider_label, "Codex");
        assert_eq!(payload.remaining_percent, Some(98));
        assert_eq!(payload.usage_label, "剩余用量");
        assert_eq!(payload.open_label, "打开详情");
        assert_eq!(payload.status_label, "自动更新已关闭");
        assert_eq!(payload.theme, "system");

        display.language = TrayLanguage::En;
        display.auto_refresh_enabled = true;
        display.auto_refresh_minutes = 12;
        let payload = tray_status_payload(&display);
        assert_eq!(payload.usage_label, "Remaining usage");
        assert_eq!(payload.open_label, "Open details");
        assert_eq!(payload.status_label, "Updates every 12 min");
        assert_eq!(payload.language, "en");
    }

    #[test]
    fn tray_status_window_stays_inside_bottom_taskbar_monitor() {
        let position = tray_status_window_position(
            PhysicalPosition::new(1800.0, 1040.0),
            PhysicalSize::new(24, 24),
            PhysicalSize::new(320, 156),
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(1920, 1080),
        );

        assert_eq!(position, PhysicalPosition::new(1592, 876));
    }

    #[test]
    fn tray_status_window_has_default_capability() {
        let capability: Value = serde_json::from_str(include_str!("../capabilities/default.json"))
            .expect("default capability should be valid JSON");
        let windows = capability["windows"]
            .as_array()
            .expect("default capability should list windows");

        assert!(windows
            .iter()
            .any(|window| window.as_str() == Some(TRAY_STATUS_WINDOW_LABEL)));
    }

    #[test]
    fn tray_status_window_is_not_in_cross_platform_config() {
        let config: Value = serde_json::from_str(include_str!("../tauri.conf.json"))
            .expect("Tauri config should be valid JSON");
        let windows = config["app"]["windows"]
            .as_array()
            .expect("Tauri config should list windows");

        assert!(windows
            .iter()
            .all(|window| window["label"].as_str() != Some(TRAY_STATUS_WINDOW_LABEL)));
    }

    #[test]
    fn windows_icons_match_common_dpi_scales() {
        assert_eq!(windows_icon_size(16, 1.0), 16);
        assert_eq!(windows_icon_size(16, 1.25), 20);
        assert_eq!(windows_icon_size(16, 1.5), 24);
        assert_eq!(windows_icon_size(16, 1.75), 28);
        assert_eq!(windows_icon_size(16, 2.0), 32);
        assert_eq!(windows_icon_size(32, 1.0), 32);
        assert_eq!(windows_icon_size(32, 1.25), 40);
        assert_eq!(windows_icon_size(32, 1.5), 48);
        assert_eq!(windows_icon_size(32, 1.75), 56);
        assert_eq!(windows_icon_size(32, 2.0), 64);
    }

    #[test]
    fn tray_remaining_percent_prefers_primary_window_and_rounds() {
        let rate_limits = RateLimits {
            updated_at: None,
            plan_type: None,
            reached_type: None,
            windows: vec![
                test_rate_limit_window("secondary", 88.0),
                test_rate_limit_window("primary", 42.6),
            ],
        };

        assert_eq!(tray_remaining_percent(Some(&rate_limits)), Some(43));
    }

    #[test]
    fn tray_remaining_percent_falls_back_to_first_window_and_clamps() {
        let rate_limits = RateLimits {
            updated_at: None,
            plan_type: None,
            reached_type: None,
            windows: vec![test_rate_limit_window("rolling", 104.0)],
        };

        assert_eq!(tray_remaining_percent(Some(&rate_limits)), Some(100));
        assert_eq!(tray_remaining_percent(None), None);
    }

    #[test]
    fn tray_display_rejects_usage_from_inactive_provider() {
        let mut display = test_tray_display();

        assert!(!display.update_usage("claude", Some(90)));
        assert_eq!(display.remaining_percent, Some(50));
        assert!(display.update_usage("codex", Some(45)));
        assert_eq!(display.remaining_percent, Some(45));
    }

    #[test]
    fn tray_copy_follows_selected_language() {
        assert_eq!(tray_language("zh-CN"), TrayLanguage::Zh);
        assert_eq!(tray_language("en-US"), TrayLanguage::En);
        assert_eq!(tray_menu_labels(TrayLanguage::Zh), ("打开 Dial", "退出"));
        assert_eq!(tray_menu_labels(TrayLanguage::En), ("Open Dial", "Quit"));

        let mut display = test_tray_display();
        display.remaining_percent = Some(47);
        assert_eq!(tray_tooltip(&display), "Dial · Codex · 剩余 47%");

        display.language = TrayLanguage::En;
        assert_eq!(tray_tooltip(&display), "Dial · Codex · 47% remaining");
    }

    #[test]
    fn tray_auto_refresh_delay_follows_settings() {
        let mut settings = default_settings();
        assert_eq!(tray_auto_refresh_delay(&settings), None);

        settings.auto_refresh_enabled = true;
        settings.auto_refresh_minutes = 12;
        assert_eq!(
            tray_auto_refresh_delay(&settings),
            Some(std::time::Duration::from_secs(12 * 60))
        );
    }

    #[test]
    fn tray_status_payload_distinguishes_refreshing_and_stale_data() {
        let mut display = test_tray_display();
        display.is_refreshing = true;
        assert_eq!(tray_status_payload(&display).status_label, "正在更新...");

        display.is_refreshing = false;
        display.refresh_failed = true;
        assert_eq!(
            tray_status_payload(&display).status_label,
            "更新失败，显示上次数据"
        );
    }

    #[test]
    fn normalize_rate_limits_keeps_only_available_periods() {
        let weekly_only = normalize_rate_limits(
            Some(&json!({
                "plan_type": "prolite",
                "primary": {
                    "used_percent": 52.0,
                    "window_minutes": 10080,
                    "resets_at": 1784509607
                },
                "secondary": null
            })),
            1784019200000,
        )
        .expect("weekly rate limit should be available");

        assert_eq!(weekly_only.windows.len(), 1);
        assert_eq!(weekly_only.windows[0].label, "1 周");
        assert_eq!(weekly_only.windows[0].remaining_percent, 48.0);

        assert!(normalize_rate_limits(
            Some(&json!({
                "primary": { "used_percent": 12.0 },
                "secondary": null
            })),
            1784019200000,
        )
        .is_none());
    }

    #[test]
    fn normalize_settings_switches_active_to_enabled_provider() {
        let mut settings = default_settings();
        settings.active_provider = "cursor".to_string();
        settings.enabled_providers = vec!["claude".to_string(), "copilot".to_string()];

        let normalized = normalize_settings(settings);

        assert_eq!(normalized.enabled_providers, vec!["claude", "copilot"]);
        assert_eq!(normalized.active_provider, "claude");
    }

    #[test]
    fn normalize_settings_keeps_at_least_one_enabled_provider() {
        let mut settings = default_settings();
        settings.active_provider = "unknown".to_string();
        settings.enabled_providers = vec!["unknown".to_string()];

        let normalized = normalize_settings(settings);

        assert_eq!(normalized.enabled_providers, default_enabled_providers());
        assert_eq!(normalized.active_provider, "codex");
    }

    #[test]
    fn normalize_settings_caps_chart_days() {
        let mut settings = default_settings();
        settings.chart_days = MAX_CHART_DAYS + 1;

        let normalized = normalize_settings(settings);

        assert_eq!(normalized.chart_days, MAX_CHART_DAYS);
    }

    #[test]
    fn normalize_settings_caps_auto_refresh_minutes() {
        let mut settings = default_settings();
        settings.auto_refresh_enabled = true;
        settings.auto_refresh_minutes = MAX_AUTO_REFRESH_MINUTES + 1;

        let normalized = normalize_settings(settings);

        assert!(normalized.auto_refresh_enabled);
        assert_eq!(normalized.auto_refresh_minutes, MAX_AUTO_REFRESH_MINUTES);
    }

    #[test]
    fn normalize_settings_keeps_auto_refresh_minutes_positive() {
        let mut settings = default_settings();
        settings.auto_refresh_minutes = 0;

        let normalized = normalize_settings(settings);

        assert_eq!(normalized.auto_refresh_minutes, 1);
    }

    #[test]
    fn sqlite_nonnegative_u64_handles_null_and_negative_values() {
        assert_eq!(sqlite_nonnegative_u64(None), 0);
        assert_eq!(sqlite_nonnegative_u64(Some(-1)), 0);
        assert_eq!(sqlite_nonnegative_u64(Some(42)), 42);
    }

    #[test]
    fn rank_by_tokens_with_other_groups_items_after_top_four() {
        let ranked = rank_by_tokens_with_other(
            [
                ("model-a".to_string(), 100),
                ("model-b".to_string(), 90),
                ("model-c".to_string(), 80),
                ("model-d".to_string(), 70),
                ("model-e".to_string(), 60),
                ("model-f".to_string(), 50),
            ],
            4,
            "其他",
        );

        assert_eq!(ranked.len(), 5);
        assert_eq!(ranked[0].name, "model-a");
        assert_eq!(ranked[3].name, "model-d");
        assert_eq!(ranked[4].name, "其他");
        assert_eq!(ranked[4].value, 110);
    }

    #[test]
    fn build_stats_without_pricing_reports_tokens_without_costs() {
        let now = DateTime::parse_from_rfc3339("2026-06-30T12:00:00.000Z")
            .unwrap()
            .with_timezone(&Local);
        let timestamp_ms = DateTime::parse_from_rfc3339("2026-06-30T12:00:00.000Z")
            .unwrap()
            .timestamp_millis();
        let thread = Thread {
            id: "local-one".to_string(),
            title: "Local estimate".to_string(),
            source: "Local".to_string(),
            model: "local-model".to_string(),
            cwd: String::new(),
            archived: false,
            tokens_used: 1200,
            created_at_ms: timestamp_ms,
            updated_at_ms: timestamp_ms,
            rollout_path: String::new(),
            usage_events: vec![UsageEvent {
                thread_id: "local-one".to_string(),
                timestamp_ms,
                model: "local-model".to_string(),
                total_tokens: 1200,
                plan_type: None,
                rate_limits: None,
            }],
        };

        let stats = build_stats_from_threads(
            vec![thread],
            now,
            30,
            test_account(),
            None,
            None,
            false,
            json!({ "test": true }),
        );

        assert_eq!(stats.featured.today_tokens, 1200);
        assert_eq!(stats.featured.period_tokens, 1200);
        assert!(!stats.featured.cost_available);
        assert_eq!(stats.featured.today_cost, 0.0);
        assert_eq!(stats.featured.period_cost, 0.0);
        assert_eq!(stats.daily_series[29].cost, 0.0);
    }

    #[test]
    fn codex_usage_history_preserves_daily_tokens_after_source_shrinks() {
        let now = DateTime::parse_from_rfc3339("2026-06-30T12:00:00.000Z")
            .unwrap()
            .with_timezone(&Local);
        let timestamp_ms = DateTime::parse_from_rfc3339("2026-06-30T12:00:00.000Z")
            .unwrap()
            .timestamp_millis();
        let home = env::temp_dir().join("ai-usage-codex-history-home-a");
        let thread = Thread {
            id: "codex-one".to_string(),
            title: "Codex estimate".to_string(),
            source: "Codex".to_string(),
            model: "gpt-5-codex".to_string(),
            cwd: String::new(),
            archived: false,
            tokens_used: 1200,
            created_at_ms: timestamp_ms,
            updated_at_ms: timestamp_ms,
            rollout_path: String::new(),
            usage_events: vec![UsageEvent {
                thread_id: "codex-one".to_string(),
                timestamp_ms,
                model: "gpt-5-codex".to_string(),
                total_tokens: 1200,
                plan_type: None,
                rate_limits: None,
            }],
        };
        let mut history = UsageHistory::default();
        let mut first_stats = build_stats_from_threads(
            vec![thread],
            now,
            7,
            test_account(),
            Some(codex_pricing()),
            Some(CODEX_USD_PER_MILLION_TOKENS),
            true,
            json!({ "test": true }),
        );

        assert!(apply_codex_usage_history(
            &mut history,
            &home,
            &mut first_stats,
            CODEX_USD_PER_MILLION_TOKENS,
        ));

        let mut second_stats = build_stats_from_threads(
            Vec::new(),
            now,
            7,
            test_account(),
            Some(codex_pricing()),
            Some(CODEX_USD_PER_MILLION_TOKENS),
            true,
            json!({ "test": true }),
        );

        assert!(!apply_codex_usage_history(
            &mut history,
            &home,
            &mut second_stats,
            CODEX_USD_PER_MILLION_TOKENS,
        ));

        assert_eq!(second_stats.featured.today_tokens, 1200);
        assert_eq!(second_stats.featured.period_tokens, 1200);
        assert_eq!(second_stats.featured.latest_token_usage, 1200);
        assert_eq!(second_stats.totals.total_tokens, 1200);
        assert_eq!(second_stats.daily_series.last().unwrap().tokens, 1200);
    }

    #[test]
    fn codex_usage_history_is_scoped_by_home() {
        let now = DateTime::parse_from_rfc3339("2026-06-30T12:00:00.000Z")
            .unwrap()
            .with_timezone(&Local);
        let timestamp_ms = DateTime::parse_from_rfc3339("2026-06-30T12:00:00.000Z")
            .unwrap()
            .timestamp_millis();
        let first_home = env::temp_dir().join("ai-usage-codex-history-home-a");
        let second_home = env::temp_dir().join("ai-usage-codex-history-home-b");
        let thread = Thread {
            id: "codex-one".to_string(),
            title: "Codex estimate".to_string(),
            source: "Codex".to_string(),
            model: "gpt-5-codex".to_string(),
            cwd: String::new(),
            archived: false,
            tokens_used: 900,
            created_at_ms: timestamp_ms,
            updated_at_ms: timestamp_ms,
            rollout_path: String::new(),
            usage_events: vec![UsageEvent {
                thread_id: "codex-one".to_string(),
                timestamp_ms,
                model: "gpt-5-codex".to_string(),
                total_tokens: 900,
                plan_type: None,
                rate_limits: None,
            }],
        };
        let mut history = UsageHistory::default();
        let mut first_stats = build_stats_from_threads(
            vec![thread],
            now,
            7,
            test_account(),
            Some(codex_pricing()),
            Some(CODEX_USD_PER_MILLION_TOKENS),
            true,
            json!({ "test": true }),
        );
        apply_codex_usage_history(
            &mut history,
            &first_home,
            &mut first_stats,
            CODEX_USD_PER_MILLION_TOKENS,
        );

        let mut second_stats = build_stats_from_threads(
            Vec::new(),
            now,
            7,
            test_account(),
            Some(codex_pricing()),
            Some(CODEX_USD_PER_MILLION_TOKENS),
            true,
            json!({ "test": true }),
        );
        apply_codex_usage_history(
            &mut history,
            &second_home,
            &mut second_stats,
            CODEX_USD_PER_MILLION_TOKENS,
        );

        assert_eq!(second_stats.featured.period_tokens, 0);
        assert_eq!(second_stats.totals.total_tokens, 0);
    }

    #[test]
    fn quarantine_invalid_settings_renames_original_file() {
        let settings_file = temp_jsonl_path("settings.json");
        let settings_dir = settings_file.parent().unwrap().to_path_buf();
        fs::write(&settings_file, "{ invalid json").expect("invalid settings should be written");

        quarantine_invalid_settings(&settings_file);

        assert!(!settings_file.exists());
        let backups = fs::read_dir(&settings_dir)
            .expect("settings dir should be readable")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("settings.invalid-")
            })
            .count();
        assert_eq!(backups, 1);

        let _ = fs::remove_dir_all(settings_dir);
    }

    #[test]
    fn unique_settings_temp_path_avoids_reusing_same_name() {
        let settings_file = temp_jsonl_path("settings.json");
        let first = unique_settings_temp_path(&settings_file);
        let second = unique_settings_temp_path(&settings_file);

        assert_ne!(first, second);
        assert_eq!(first.parent(), settings_file.parent());
        assert_eq!(second.parent(), settings_file.parent());

        let _ = fs::remove_dir_all(settings_file.parent().unwrap());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn replace_settings_file_overwrites_existing_file() {
        let settings_file = temp_jsonl_path("settings.json");
        let temp_file = unique_settings_temp_path(&settings_file);
        fs::write(&settings_file, "old").expect("old settings should be written");
        fs::write(&temp_file, "new").expect("new settings should be written");

        replace_settings_file(&temp_file, &settings_file).expect("settings should be replaced");

        assert_eq!(
            fs::read_to_string(&settings_file).expect("settings should be readable"),
            "new"
        );
        assert!(!temp_file.exists());

        let _ = fs::remove_dir_all(settings_file.parent().unwrap());
    }

    fn temp_jsonl_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let counter = TEST_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = env::temp_dir().join(format!(
            "ai-usage-test-{}-{nonce}-{counter}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("test temp dir should be created");
        dir.join(name)
    }

    #[test]
    fn source_label_normalizes_known_sources() {
        assert_eq!(
            source_label(Some(r#"{"subagent":{"other":"guardian"}}"#)),
            "子任务"
        );
        assert_eq!(source_label(Some("vscode")), "VS Code");
        assert_eq!(source_label(Some("")), "Unknown");
        assert_eq!(source_label(None), "Unknown");
    }

    #[test]
    fn usage_total_includes_claude_cache_and_output_tokens() {
        let usage = json!({
          "input_tokens": 10,
          "cache_creation_input_tokens": 20,
          "cache_read_input_tokens": 30,
          "output_tokens": 40
        });

        assert_eq!(usage_total(&usage), 100);
    }

    #[test]
    fn read_codex_usage_events_uses_cumulative_deltas_and_keeps_latest_limits() {
        let file_path = temp_jsonl_path("rollout.jsonl");
        let rows = [
            json!({
              "timestamp": "2026-06-30T10:00:00.000Z",
              "type": "event_msg",
              "payload": {
                "type": "token_count",
                "info": null,
                "rate_limits": {
                  "plan_type": "prolite",
                  "primary": { "used_percent": 12, "window_minutes": 10080, "resets_at": 1783478400 },
                  "secondary": null
                }
              }
            }),
            json!({
              "timestamp": "2026-06-30T10:01:00.000Z",
              "type": "event_msg",
              "payload": {
                "type": "token_count",
                "info": {
                  "total_token_usage": { "total_tokens": 100 },
                  "last_token_usage": { "total_tokens": 100 }
                },
                "rate_limits": {
                  "plan_type": "prolite",
                  "primary": { "used_percent": 13, "window_minutes": 10080, "resets_at": 1783478400 },
                  "secondary": null
                }
              }
            }),
            json!({
              "timestamp": "2026-06-30T10:02:00.000Z",
              "type": "event_msg",
              "payload": {
                "type": "token_count",
                "info": {
                  "total_token_usage": { "total_tokens": 100 },
                  "last_token_usage": { "total_tokens": 100 }
                },
                "rate_limits": {
                  "plan_type": "prolite",
                  "primary": { "used_percent": 14, "window_minutes": 10080, "resets_at": 1783478400 },
                  "secondary": null
                }
              }
            }),
            json!({
              "timestamp": "2026-06-30T10:03:00.000Z",
              "type": "event_msg",
              "payload": {
                "type": "token_count",
                "info": {
                  "total_token_usage": { "total_tokens": 150 },
                  "last_token_usage": { "total_tokens": 50 }
                },
                "rate_limits": {
                  "plan_type": "prolite",
                  "primary": { "used_percent": 15, "window_minutes": 10080, "resets_at": 1783478400 },
                  "secondary": null
                }
              }
            }),
        ];
        let mut file = File::create(&file_path).expect("test jsonl should be created");
        for row in rows {
            writeln!(file, "{row}").expect("test jsonl row should be written");
        }

        let mut thread = Thread {
            id: "codex-one".to_string(),
            title: "Codex rollout".to_string(),
            source: "Codex".to_string(),
            model: "gpt-5.5".to_string(),
            cwd: "/work/app".to_string(),
            archived: false,
            tokens_used: 150,
            created_at_ms: DateTime::parse_from_rfc3339("2026-06-30T09:00:00.000Z")
                .unwrap()
                .timestamp_millis(),
            updated_at_ms: DateTime::parse_from_rfc3339("2026-06-30T10:03:00.000Z")
                .unwrap()
                .timestamp_millis(),
            rollout_path: file_path.to_string_lossy().to_string(),
            usage_events: Vec::new(),
        };

        let events = read_codex_usage_events(&[thread.clone()]);
        assert_eq!(events.len(), 4);
        assert_eq!(
            events.iter().map(|event| event.total_tokens).sum::<u64>(),
            150
        );
        assert_eq!(events[0].total_tokens, 0);
        assert_eq!(events[1].total_tokens, 100);
        assert_eq!(events[2].total_tokens, 0);
        assert_eq!(events[3].total_tokens, 50);

        thread.usage_events = events;
        let now = DateTime::parse_from_rfc3339("2026-06-30T12:00:00.000Z")
            .unwrap()
            .with_timezone(&Local);
        let stats = build_stats_from_threads(
            vec![thread],
            now,
            30,
            test_account(),
            None,
            Some(CODEX_USD_PER_MILLION_TOKENS),
            true,
            json!({ "test": true }),
        );

        assert_eq!(stats.featured.period_tokens, 150);
        assert_eq!(stats.rate_limits.as_ref().unwrap().windows.len(), 1);
        assert_eq!(stats.rate_limits.as_ref().unwrap().windows[0].label, "1 周");
        assert_eq!(
            stats.rate_limits.as_ref().unwrap().windows[0].remaining_percent,
            85.0
        );

        let _ = fs::remove_dir_all(file_path.parent().unwrap());
    }

    #[test]
    fn bind_codex_usage_events_keeps_threads_that_share_a_rollout() {
        let rollout_path = PathBuf::from("shared-rollout.jsonl");
        let thread = |id: &str, model: &str| Thread {
            id: id.to_string(),
            title: id.to_string(),
            source: "Codex".to_string(),
            model: model.to_string(),
            cwd: "/work/app".to_string(),
            archived: false,
            tokens_used: 10,
            created_at_ms: 1,
            updated_at_ms: 2,
            rollout_path: rollout_path.to_string_lossy().to_string(),
            usage_events: Vec::new(),
        };
        let rollout_threads = HashMap::from([(
            rollout_path.clone(),
            vec![thread("one", "gpt-one"), thread("two", "gpt-two")],
        )]);
        let scanned_files = vec![ScannedFile {
            path: rollout_path,
            values: vec![UsageEvent {
                thread_id: "cached".to_string(),
                timestamp_ms: 3,
                model: "cached-model".to_string(),
                total_tokens: 10,
                plan_type: None,
                rate_limits: None,
            }],
        }];

        let events = bind_codex_usage_events(scanned_files, &rollout_threads);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].thread_id, "one");
        assert_eq!(events[0].model, "gpt-one");
        assert_eq!(events[1].thread_id, "two");
        assert_eq!(events[1].model, "gpt-two");
    }

    #[test]
    fn build_stats_from_threads_aggregates_codex_usage_events() {
        let now = DateTime::parse_from_rfc3339("2026-06-30T12:00:00.000Z")
            .unwrap()
            .with_timezone(&Local);
        let rate_limits = normalize_rate_limits(
            Some(&json!({
              "plan_type": "prolite",
              "primary": { "used_percent": 8, "window_minutes": 10080, "resets_at": 1783478400 },
              "secondary": null
            })),
            DateTime::parse_from_rfc3339("2026-06-30T11:00:00.000Z")
                .unwrap()
                .timestamp_millis(),
        );
        let threads = vec![
            Thread {
                id: "one".to_string(),
                title: "First".to_string(),
                source: source_label(Some("vscode")),
                model: "gpt-5.5".to_string(),
                cwd: "/work/app-one".to_string(),
                archived: false,
                tokens_used: 1200,
                created_at_ms: DateTime::parse_from_rfc3339("2026-06-29T10:00:00.000Z")
                    .unwrap()
                    .timestamp_millis(),
                updated_at_ms: DateTime::parse_from_rfc3339("2026-06-30T09:00:00.000Z")
                    .unwrap()
                    .timestamp_millis(),
                rollout_path: String::new(),
                usage_events: vec![UsageEvent {
                    thread_id: "one".to_string(),
                    timestamp_ms: DateTime::parse_from_rfc3339("2026-06-30T11:00:00.000Z")
                        .unwrap()
                        .timestamp_millis(),
                    model: "gpt-5.5".to_string(),
                    total_tokens: 1500,
                    plan_type: Some("prolite".to_string()),
                    rate_limits,
                }],
            },
            Thread {
                id: "two".to_string(),
                title: "Second".to_string(),
                source: source_label(Some(r#"{"subagent":{"other":"guardian"}}"#)),
                model: "codex-auto-review".to_string(),
                cwd: "/work/app-two".to_string(),
                archived: true,
                tokens_used: 300,
                created_at_ms: DateTime::parse_from_rfc3339("2026-06-20T10:00:00.000Z")
                    .unwrap()
                    .timestamp_millis(),
                updated_at_ms: DateTime::parse_from_rfc3339("2026-06-21T09:00:00.000Z")
                    .unwrap()
                    .timestamp_millis(),
                rollout_path: String::new(),
                usage_events: vec![UsageEvent {
                    thread_id: "two".to_string(),
                    timestamp_ms: DateTime::parse_from_rfc3339("2026-06-20T11:00:00.000Z")
                        .unwrap()
                        .timestamp_millis(),
                    model: "codex-auto-review".to_string(),
                    total_tokens: 200,
                    plan_type: None,
                    rate_limits: None,
                }],
            },
        ];

        let stats = build_stats_from_threads(
            threads,
            now,
            30,
            test_account(),
            None,
            Some(CODEX_USD_PER_MILLION_TOKENS),
            true,
            json!({ "test": true }),
        );

        assert_eq!(stats.totals.threads, 2);
        assert_eq!(stats.totals.active_threads, 1);
        assert_eq!(stats.totals.archived_threads, 1);
        assert_eq!(stats.totals.total_tokens, 1700);
        assert_eq!(stats.featured.period_tokens, 1700);
        assert_eq!(stats.featured.period_cost, 0.0017);
        assert_eq!(stats.featured.latest_token_usage, 1500);
        assert!(stats.featured.cost_estimated_from_token_events);
        assert_eq!(stats.rate_limits.as_ref().unwrap().windows[0].label, "1 周");
        assert_eq!(
            stats.rate_limits.as_ref().unwrap().windows[0].remaining_percent,
            92.0
        );
        assert_eq!(stats.models[0].name, "gpt-5.5");
        assert!(stats
            .sources
            .iter()
            .any(|source| source.name == "VS Code" && source.value == 1));
        assert!(stats
            .sources
            .iter()
            .any(|source| source.name == "子任务" && source.value == 1));
        assert_eq!(stats.daily_series.len(), 30);
        assert_eq!(stats.daily_series[28].threads, 1);
        assert_eq!(stats.latest_threads[0].id, "one");
        assert_eq!(stats.latest_threads[0].tokens_used, 1500);
    }

    #[test]
    fn scan_candidate_files_limits_visited_files_before_matching() {
        let root = temp_jsonl_path("scan-root");
        fs::create_dir_all(&root).expect("scan root should be created");
        fs::write(root.join("notes.txt"), "not a data file").expect("non-candidate should exist");
        fs::write(root.join("copilot-chat.jsonl"), "{}").expect("candidate should exist");

        let files = scan_candidate_files(&root, is_copilot_data_file)
            .expect("fixture enumeration should be complete");

        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].file_name().and_then(|name| name.to_str()),
            Some("copilot-chat.jsonl")
        );

        let _ = fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn unbounded_candidate_scan_keeps_deep_claude_sessions() {
        let root = temp_jsonl_path("claude-projects");
        fs::create_dir_all(&root).expect("scan root should be created");
        let mut nested = root.clone();
        for depth in 0..DEFAULT_SCAN_MAX_DEPTH {
            nested = nested.join(format!("level-{depth}"));
        }
        fs::create_dir_all(&nested).expect("nested fixture should be created");
        let session = nested.join("session.jsonl");
        fs::write(&session, "{}").expect("deep Claude session should exist");

        let limited = scan_candidate_files(&root, is_claude_session_file)
            .expect("depth limiting is not an enumeration failure");
        let unbounded = scan_all_candidate_files(&root, is_claude_session_file)
            .expect("fixture enumeration should be complete");

        assert!(limited.is_empty());
        assert_eq!(unbounded, vec![session]);

        let _ = fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn chatgpt_sqlite_candidates_require_chatgpt_context() {
        assert!(!is_chatgpt_data_file(Path::new("/tmp/random/cache.db")));
        assert!(is_chatgpt_data_file(Path::new(
            "/tmp/com.openai.chat/workspace/conversations.db"
        )));
    }

    #[test]
    fn read_claude_session_deduplicates_repeated_assistant_updates() {
        let file_path = temp_jsonl_path("session-one.jsonl");
        let rows = [
            json!({
              "type": "custom-title",
              "customTitle": "Dedup session",
              "sessionId": "session-one"
            }),
            json!({
              "type": "user",
              "timestamp": "2026-06-30T10:00:00.000Z",
              "sessionId": "session-one",
              "cwd": "/work/app",
              "entrypoint": "cli",
              "message": { "role": "user", "content": "hello" }
            }),
            json!({
              "type": "assistant",
              "timestamp": "2026-06-30T10:00:10.000Z",
              "sessionId": "session-one",
              "cwd": "/work/app",
              "message": {
                "id": "msg-one",
                "model": "claude-opus-4-8",
                "usage": {
                  "input_tokens": 10,
                  "cache_creation_input_tokens": 20,
                  "cache_read_input_tokens": 30,
                  "output_tokens": 40
                }
              }
            }),
            json!({
              "type": "assistant",
              "timestamp": "2026-06-30T10:00:12.000Z",
              "sessionId": "session-one",
              "cwd": "/work/app",
              "message": {
                "id": "msg-one",
                "model": "claude-opus-4-8",
                "usage": {
                  "input_tokens": 10,
                  "cache_creation_input_tokens": 20,
                  "cache_read_input_tokens": 30,
                  "output_tokens": 40
                }
              }
            }),
        ];
        let mut file = File::create(&file_path).expect("test jsonl should be created");
        for row in rows {
            writeln!(file, "{row}").expect("test jsonl row should be written");
        }

        let session = read_claude_session(&file_path).expect("session should parse");

        assert_eq!(session.id, "session-one");
        assert_eq!(session.title, "Dedup session");
        assert_eq!(session.source, "CLI");
        assert_eq!(session.tokens_used, 100);
        assert_eq!(session.usage_events.len(), 1);

        let _ = fs::remove_dir_all(file_path.parent().unwrap());
    }

    #[test]
    fn read_claude_account_uses_latest_telemetry_identity() {
        let telemetry_file = temp_jsonl_path("events.json");
        let home = telemetry_file.parent().unwrap().to_path_buf();
        let telemetry_dir = home.join("telemetry");
        fs::create_dir_all(&telemetry_dir).expect("telemetry dir should be created");
        let telemetry_file = telemetry_dir.join("events.json");
        let rows = [
            json!({
              "event_data": {
                "client_timestamp": "2026-06-30T09:00:00.000Z",
                "email": "old@example.com",
                "user_type": "external",
                "env": { "is_claude_ai_auth": true }
              }
            }),
            json!({
              "event_data": {
                "client_timestamp": "2026-06-30T10:00:00.000Z",
                "email": "new@example.com",
                "user_type": "external",
                "env": { "is_claude_ai_auth": true }
              }
            }),
        ];
        let mut file = File::create(&telemetry_file).expect("telemetry file should be created");
        for row in rows {
            writeln!(file, "{row}").expect("telemetry row should be written");
        }

        let account = read_claude_account(&home);

        assert_eq!(account.display_name, "new@example.com");
        assert_eq!(account.initials, "N");
        assert_eq!(account.plan_type.as_deref(), Some("external"));
        assert_eq!(account.plan_label, "Claude.ai account");

        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn read_claude_account_prefers_subscription_plan_over_user_type() {
        let telemetry_file = temp_jsonl_path("events.json");
        let home = telemetry_file.parent().unwrap().to_path_buf();
        let telemetry_dir = home.join("telemetry");
        fs::create_dir_all(&telemetry_dir).expect("telemetry dir should be created");
        let telemetry_file = telemetry_dir.join("events.json");
        let row = json!({
          "event_data": {
            "client_timestamp": "2026-06-30T10:00:00.000Z",
            "email": "paid@example.com",
            "user_type": "external",
            "subscription_type": "max",
            "env": { "is_claude_ai_auth": true }
          }
        });
        let mut file = File::create(&telemetry_file).expect("telemetry file should be created");
        writeln!(file, "{row}").expect("telemetry row should be written");

        let account = read_claude_account(&home);

        assert_eq!(account.display_name, "paid@example.com");
        assert_eq!(account.plan_type.as_deref(), Some("max"));
        assert_eq!(account.plan_label, "Claude Max");

        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn read_claude_account_keeps_older_plan_when_latest_identity_has_no_plan() {
        let telemetry_file = temp_jsonl_path("events.json");
        let home = telemetry_file.parent().unwrap().to_path_buf();
        let telemetry_dir = home.join("telemetry");
        fs::create_dir_all(&telemetry_dir).expect("telemetry dir should be created");
        let telemetry_file = telemetry_dir.join("events.json");
        let rows = [
            json!({
              "event_data": {
                "client_timestamp": "2026-06-30T09:00:00.000Z",
                "email": "paid@example.com",
                "user_type": "external",
                "organizationRateLimitTier": "default_claude_max_5x",
                "env": { "is_claude_ai_auth": true }
              }
            }),
            json!({
              "event_data": {
                "client_timestamp": "2026-06-30T10:00:00.000Z",
                "email": "latest@example.com",
                "user_type": "external",
                "env": { "is_claude_ai_auth": true }
              }
            }),
        ];
        let mut file = File::create(&telemetry_file).expect("telemetry file should be created");
        for row in rows {
            writeln!(file, "{row}").expect("telemetry row should be written");
        }

        let account = read_claude_account(&home);

        assert_eq!(account.display_name, "latest@example.com");
        assert_eq!(account.plan_type.as_deref(), Some("default_claude_max_5x"));
        assert_eq!(account.plan_label, "Claude Max 5x");

        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn read_claude_account_uses_config_rate_limit_tier() {
        let home = temp_jsonl_path("claude-home");
        fs::create_dir_all(&home).expect("claude home should be created");
        let config_path = home.parent().unwrap().join(".claude.json");
        fs::write(
            config_path,
            json!({
              "primaryAccount": {
                "organizationRateLimitTier": "default_claude_max_5x",
                "billingType": "apple_subscription"
              }
            })
            .to_string(),
        )
        .expect("claude config should be written");

        let account = read_claude_account(&home);

        assert_eq!(account.plan_type.as_deref(), Some("default_claude_max_5x"));
        assert_eq!(account.plan_label, "Claude Max 5x");

        let _ = fs::remove_dir_all(home.parent().unwrap());
    }

    #[test]
    fn build_claude_estimated_rate_limits_uses_recent_token_windows() {
        let now = DateTime::parse_from_rfc3339("2026-06-30T12:00:00.000Z")
            .unwrap()
            .with_timezone(&Local);
        let threads = vec![Thread {
            id: "claude-one".to_string(),
            title: "Claude".to_string(),
            source: "CLI".to_string(),
            model: "claude-opus-4-8".to_string(),
            cwd: "/work/app".to_string(),
            archived: false,
            tokens_used: 11_000,
            created_at_ms: DateTime::parse_from_rfc3339("2026-06-30T08:00:00.000Z")
                .unwrap()
                .timestamp_millis(),
            updated_at_ms: DateTime::parse_from_rfc3339("2026-06-30T11:00:00.000Z")
                .unwrap()
                .timestamp_millis(),
            rollout_path: String::new(),
            usage_events: vec![
                UsageEvent {
                    thread_id: "claude-one".to_string(),
                    timestamp_ms: DateTime::parse_from_rfc3339("2026-06-30T11:00:00.000Z")
                        .unwrap()
                        .timestamp_millis(),
                    model: "claude-opus-4-8".to_string(),
                    total_tokens: 10_000,
                    plan_type: None,
                    rate_limits: None,
                },
                UsageEvent {
                    thread_id: "claude-one".to_string(),
                    timestamp_ms: DateTime::parse_from_rfc3339("2026-06-30T01:00:00.000Z")
                        .unwrap()
                        .timestamp_millis(),
                    model: "claude-opus-4-8".to_string(),
                    total_tokens: 1_000,
                    plan_type: None,
                    rate_limits: None,
                },
            ],
        }];

        let limits =
            build_claude_estimated_rate_limits(&threads, now).expect("limits should exist");

        assert_eq!(limits.windows.len(), 2);
        assert_eq!(limits.windows[0].window_minutes, 300);
        assert_eq!(limits.windows[0].used_percent, 2.0);
        assert_eq!(limits.windows[0].remaining_percent, 98.0);
        assert_eq!(limits.windows[1].window_minutes, 10080);
        assert!((limits.windows[1].used_percent - 0.44).abs() < f64::EPSILON);
    }

    #[test]
    fn read_copilot_jsonish_file_uses_usage_and_text_estimates() {
        let file_path = temp_jsonl_path("copilot-chat.jsonl");
        let rows = [
            json!({
              "sessionId": "copilot-one",
              "title": "Copilot session",
              "workspaceFolder": "/work/copilot-app"
            }),
            json!({
              "role": "assistant",
              "timestamp": "2026-06-30T10:00:00.000Z",
              "model": "gpt-4.1",
              "content": "Here is the answer.",
              "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5
              }
            }),
            json!({
              "role": "user",
              "timestamp": "2026-06-30T10:00:05.000Z",
              "content": "hello world"
            }),
        ];
        let mut file = File::create(&file_path).expect("test jsonl should be created");
        for row in rows {
            writeln!(file, "{row}").expect("test jsonl row should be written");
        }

        let session = read_copilot_jsonish_file(&file_path).expect("copilot session should parse");

        assert_eq!(session.id, "copilot-one");
        assert_eq!(session.title, "Copilot session");
        assert_eq!(session.source, "VS Code");
        assert_eq!(session.model, "gpt-4.1");
        assert_eq!(session.cwd, "/work/copilot-app");
        assert_eq!(session.tokens_used, 18);
        assert_eq!(session.usage_events.len(), 2);

        let _ = fs::remove_dir_all(file_path.parent().unwrap());
    }

    #[test]
    fn read_chatgpt_json_export_uses_content_parts() {
        let file_path = temp_jsonl_path("conversations.json");
        let conversation = json!([
          {
            "id": "chatgpt-one",
            "title": "ChatGPT session",
            "create_time": 1782813600,
            "update_time": 1782817200,
            "mapping": {
              "user-message": {
                "message": {
                  "author": { "role": "user" },
                  "create_time": 1782813600,
                  "content": { "content_type": "text", "parts": ["Please summarize this repo."] }
                }
              },
              "assistant-message": {
                "message": {
                  "author": { "role": "assistant" },
                  "create_time": 1782817200,
                  "metadata": { "model_slug": "gpt-5.5" },
                  "content": { "content_type": "text", "parts": ["Here is a concise summary."] }
                }
              }
            }
          }
        ]);
        let mut file = File::create(&file_path).expect("chatgpt export should be created");
        write!(file, "{conversation}").expect("chatgpt export should be written");

        let sessions = read_chatgpt_jsonish_file(&file_path);

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "chatgpt-one");
        assert_eq!(sessions[0].title, "ChatGPT session");
        assert_eq!(sessions[0].source, "ChatGPT");
        assert!(sessions[0].tokens_used > 0);
        assert_eq!(sessions[0].usage_events.len(), 2);

        let _ = fs::remove_dir_all(file_path.parent().unwrap());
    }

    #[test]
    fn read_chatgpt_data_file_uses_desktop_cache_metadata() {
        let file_path = temp_jsonl_path("conversation.data");
        let conversations_dir = file_path
            .parent()
            .unwrap()
            .join("conversations-v3-user")
            .join("chatgpt-cache-one.data");
        fs::create_dir_all(conversations_dir.parent().unwrap())
            .expect("conversation cache dir should be created");
        let mut file = File::create(&conversations_dir).expect("chatgpt data file should exist");
        file.write_all(&[42_u8; 128])
            .expect("chatgpt data should be written");

        let session =
            read_chatgpt_data_file(&conversations_dir).expect("chatgpt data session should parse");

        assert_eq!(session.id, "chatgpt-cache-one");
        assert_eq!(session.source, "ChatGPT Desktop");
        assert_eq!(session.model, "ChatGPT");
        assert_eq!(session.tokens_used, 32);
        assert_eq!(session.usage_events.len(), 1);
        assert!(is_chatgpt_data_file(&conversations_dir));

        let _ = fs::remove_dir_all(file_path.parent().unwrap());
    }

    #[test]
    fn read_chatgpt_account_uses_user_scoped_directory() {
        let root = temp_jsonl_path("root");
        fs::create_dir_all(root.join("workspace-data").join("user-local123__workspace"))
            .expect("chatgpt user-scoped directory should be created");

        let account = read_chatgpt_account(std::slice::from_ref(&root));

        assert_eq!(account.display_name, "ChatGPT user local123");
        assert_eq!(account.initials, "CG");
        assert_eq!(account.plan_label, "ChatGPT");

        let _ = fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn build_chatgpt_activity_summary_counts_recent_activity() {
        let now = DateTime::parse_from_rfc3339("2026-06-30T12:00:00.000Z")
            .unwrap()
            .with_timezone(&Local);
        let recent_ms = DateTime::parse_from_rfc3339("2026-06-30T11:00:00.000Z")
            .unwrap()
            .timestamp_millis();
        let older_ms = DateTime::parse_from_rfc3339("2026-06-29T11:00:00.000Z")
            .unwrap()
            .timestamp_millis();
        let threads = vec![
            Thread {
                id: "chatgpt-one".to_string(),
                title: "Recent".to_string(),
                source: "ChatGPT Desktop".to_string(),
                model: "ChatGPT".to_string(),
                cwd: String::new(),
                archived: false,
                tokens_used: 32,
                created_at_ms: recent_ms,
                updated_at_ms: recent_ms,
                rollout_path: String::new(),
                usage_events: vec![UsageEvent {
                    thread_id: "chatgpt-one".to_string(),
                    timestamp_ms: recent_ms,
                    model: "ChatGPT".to_string(),
                    total_tokens: 32,
                    plan_type: None,
                    rate_limits: None,
                }],
            },
            Thread {
                id: "chatgpt-two".to_string(),
                title: "Older".to_string(),
                source: "ChatGPT Desktop".to_string(),
                model: "ChatGPT".to_string(),
                cwd: String::new(),
                archived: false,
                tokens_used: 12,
                created_at_ms: older_ms,
                updated_at_ms: older_ms,
                rollout_path: String::new(),
                usage_events: vec![UsageEvent {
                    thread_id: "chatgpt-two".to_string(),
                    timestamp_ms: older_ms,
                    model: "ChatGPT".to_string(),
                    total_tokens: 12,
                    plan_type: None,
                    rate_limits: None,
                }],
            },
        ];

        let activity =
            build_chatgpt_activity_summary(&threads, now).expect("activity should exist");

        assert_eq!(activity.windows.len(), 2);
        assert_eq!(activity.windows[0].window_minutes, 180);
        assert_eq!(activity.windows[0].count, 1);
        assert_eq!(activity.windows[1].window_minutes, 10080);
        assert_eq!(activity.windows[1].count, 2);
    }

    #[test]
    fn read_github_copilot_account_uses_vscode_state() {
        let db_path = temp_jsonl_path("state.vscdb");
        let global_storage = db_path.parent().unwrap().to_path_buf();
        let copilot_storage = global_storage.join("github.copilot-chat");
        fs::create_dir_all(&copilot_storage).expect("copilot storage should be created");
        let connection = Connection::open(&db_path).expect("state db should be created");
        connection
            .execute(
                "create table ItemTable (key text primary key, value text)",
                [],
            )
            .expect("item table should be created");
        connection
            .execute(
                "insert into ItemTable (key, value) values (?1, ?2)",
                rusqlite::params![
                    "github-octocat",
                    r#"[{"id":"vscode.github","allowed":true}]"#
                ],
            )
            .expect("github account row should be inserted");
        connection
            .execute(
                "insert into ItemTable (key, value) values (?1, ?2)",
                rusqlite::params![
                    "GitHub.copilot-chat",
                    r#"{"exp.github.copilot.sku":"free_limited_copilot"}"#
                ],
            )
            .expect("copilot sku row should be inserted");

        let account = read_github_copilot_account(&[copilot_storage]);

        assert_eq!(account.display_name, "octocat");
        assert_eq!(account.initials, "O");
        assert_eq!(account.plan_type.as_deref(), Some("free_limited_copilot"));
        assert_eq!(account.plan_label, "GitHub Copilot Free Limited");

        let _ = fs::remove_dir_all(global_storage);
    }

    #[test]
    fn read_cursor_account_uses_state_identity() {
        let db_path = temp_jsonl_path("state.vscdb");
        let storage = db_path.parent().unwrap().to_path_buf();
        let connection = Connection::open(&db_path).expect("state db should be created");
        connection
            .execute(
                "create table ItemTable (key text primary key, value text)",
                [],
            )
            .expect("item table should be created");
        connection
            .execute(
                "insert into ItemTable (key, value) values (?1, ?2)",
                rusqlite::params![
                    "cursor.account",
                    r#"{"email":"cursor@example.com","membershipType":"pro"}"#
                ],
            )
            .expect("cursor account row should be inserted");

        let account = read_cursor_account(std::slice::from_ref(&storage));

        assert_eq!(account.display_name, "cursor@example.com");
        assert_eq!(account.initials, "C");
        assert_eq!(account.plan_type.as_deref(), Some("pro"));
        assert_eq!(account.plan_label, "Cursor Pro");

        let _ = fs::remove_dir_all(storage);
    }

    #[test]
    fn cursor_scan_roots_include_workspace_sibling_for_configured_global_storage() {
        let user_root = temp_jsonl_path("User");
        let global_storage = user_root.join("globalStorage");
        let workspace_storage = user_root.join("workspaceStorage");

        let roots = cursor_scan_roots_for_home(&global_storage);

        assert!(roots.contains(&global_storage));
        assert!(roots.contains(&workspace_storage));

        let _ = fs::remove_dir_all(user_root.parent().unwrap());
    }

    #[test]
    fn read_cursor_account_uses_plain_state_identity() {
        let db_path = temp_jsonl_path("state.vscdb");
        let storage = db_path.parent().unwrap().to_path_buf();
        let connection = Connection::open(&db_path).expect("state db should be created");
        connection
            .execute(
                "create table ItemTable (key text primary key, value text)",
                [],
            )
            .expect("item table should be created");
        connection
            .execute(
                "insert into ItemTable (key, value) values (?1, ?2)",
                rusqlite::params!["cursorAuth/cachedEmail", "cursor-user@example.com"],
            )
            .expect("cursor email row should be inserted");
        connection
            .execute(
                "insert into ItemTable (key, value) values (?1, ?2)",
                rusqlite::params!["cursorMembershipType", "pro"],
            )
            .expect("cursor plan row should be inserted");

        let account = read_cursor_account(std::slice::from_ref(&storage));

        assert_eq!(account.display_name, "cursor-user@example.com");
        assert_eq!(account.initials, "C");
        assert_eq!(account.plan_type.as_deref(), Some("pro"));
        assert_eq!(account.plan_label, "Cursor Pro");

        let _ = fs::remove_dir_all(storage);
    }

    #[test]
    fn read_cursor_sqlite_threads_parses_nested_json_string_values() {
        let db_path = temp_jsonl_path("state.vscdb");
        let storage = db_path.parent().unwrap().to_path_buf();
        let connection = Connection::open(&db_path).expect("state db should be created");
        connection
            .execute(
                "create table ItemTable (key text primary key, value text)",
                [],
            )
            .expect("item table should be created");
        let chat_value = json!({
          "conversationId": "cursor-state-one",
          "title": "Cursor state chat",
          "workspacePath": "/work/cursor-state",
          "messages": [
            {
              "role": "assistant",
              "timestamp": "2026-06-30T10:00:00.000Z",
              "text": "Cursor response from state storage."
            }
          ]
        });
        connection
            .execute(
                "insert into ItemTable (key, value) values (?1, ?2)",
                rusqlite::params![
                    "workbench.panel.aichat.view.aichat.chatdata",
                    serde_json::to_string(&serde_json::to_string(&chat_value).unwrap()).unwrap()
                ],
            )
            .expect("cursor chat row should be inserted");

        let threads = read_cursor_sqlite_threads(&db_path);

        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "cursor-state-one");
        assert_eq!(threads[0].title, "Cursor state chat");
        assert_eq!(threads[0].tokens_used, 8);

        let _ = fs::remove_dir_all(storage);
    }

    #[test]
    fn sqlite_parser_surfaces_database_errors_for_cache_retry() {
        let db_path = temp_jsonl_path("state.vscdb");
        fs::write(&db_path, "not a sqlite database").expect("invalid database should be created");

        let result = try_read_cursor_sqlite_threads(&db_path);

        assert!(result.is_err());
        let _ = fs::remove_dir_all(db_path.parent().unwrap());
    }

    #[test]
    fn read_cursor_sqlite_threads_uses_composer_headers() {
        let db_path = temp_jsonl_path("state.vscdb");
        let storage = db_path.parent().unwrap().to_path_buf();
        let connection = Connection::open(&db_path).expect("state db should be created");
        connection
            .execute(
                "create table ItemTable (key text primary key, value text)",
                [],
            )
            .expect("item table should be created");
        let headers = json!({
          "allComposers": [
            {
              "type": "head",
              "composerId": "cursor-header-one",
              "name": "Project in-depth review",
              "createdAt": 1783043837156_i64,
              "lastUpdatedAt": 1783043837320_i64,
              "contextUsagePercent": 36.5,
              "unifiedMode": "agent",
              "workspaceIdentifier": {
                "uri": {
                  "fsPath": "/Users/example/project",
                  "path": "/Users/example/project"
                }
              }
            },
            {
              "type": "head",
              "composerId": "empty-state-draft",
              "isDraft": true,
              "createdAt": 1783043462141_i64
            }
          ]
        });
        connection
            .execute(
                "insert into ItemTable (key, value) values (?1, ?2)",
                rusqlite::params!["composer.composerHeaders", headers.to_string()],
            )
            .expect("cursor composer headers row should be inserted");

        let threads = read_cursor_sqlite_threads(&db_path);

        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "cursor-header-one");
        assert_eq!(threads[0].title, "Project in-depth review");
        assert_eq!(threads[0].source, "Cursor Composer");
        assert_eq!(threads[0].model, "Cursor agent");
        assert_eq!(threads[0].cwd, "/Users/example/project");
        assert_eq!(threads[0].tokens_used, 3650);
        assert_eq!(threads[0].usage_events.len(), 1);

        let _ = fs::remove_dir_all(storage);
    }

    #[test]
    fn read_cursor_jsonish_file_uses_cursor_labels() {
        let file_path = temp_jsonl_path("cursor-composer.jsonl");
        let rows = [
            json!({
              "conversationId": "cursor-one",
              "title": "Cursor composer",
              "workspacePath": "/work/cursor-app"
            }),
            json!({
              "role": "assistant",
              "timestamp": "2026-06-30T10:00:00.000Z",
              "content": "Cursor generated this response.",
              "usage": {
                "inputTokens": 12,
                "outputTokens": 8
              }
            }),
        ];
        let mut file = File::create(&file_path).expect("test jsonl should be created");
        for row in rows {
            writeln!(file, "{row}").expect("test jsonl row should be written");
        }

        let session = read_cursor_jsonish_file(&file_path).expect("cursor session should parse");

        assert_eq!(session.id, "cursor-one");
        assert_eq!(session.title, "Cursor composer");
        assert_eq!(session.source, "Cursor");
        assert_eq!(session.model, "Cursor");
        assert_eq!(session.cwd, "/work/cursor-app");
        assert_eq!(session.tokens_used, 20);
        assert_eq!(session.usage_events.len(), 1);

        let _ = fs::remove_dir_all(file_path.parent().unwrap());
    }
}
