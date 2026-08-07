const tauriCore = window.__TAURI__?.core;
const tauriWindow = window.__TAURI__?.window;
const REPOSITORY_URL = "https://github.com/peipeitu/dial";
const ISSUE_URL = "https://github.com/peipeitu/dial/issues";
const MIN_CHART_DAYS = 7;
const DEFAULT_CHART_DAYS = 30;
const MAX_CHART_DAYS = 90;
const DEFAULT_AUTO_REFRESH_ENABLED = false;
const DEFAULT_AUTO_REFRESH_MINUTES = 30;
const MAX_AUTO_REFRESH_MINUTES = 1440;
const MAX_STATS_CACHE_ENTRIES = 10;

function detectPlatform() {
  const platform = navigator.userAgentData?.platform || navigator.platform || "";
  const normalized = platform.toLowerCase();
  if (normalized.includes("mac")) return "darwin";
  if (normalized.includes("win")) return "windows";
  if (normalized.includes("linux")) return "linux";
  return normalized || "unknown";
}

const aiUsage = window.aiUsage || {
  platform: detectPlatform(),
  getStats: (provider) => tauriCore.invoke("get_stats", { provider }),
  rebuildStatsCache: (provider) => tauriCore.invoke("rebuild_stats_cache", { provider }),
  getScanDiagnostics: () => tauriCore.invoke("get_scan_diagnostics"),
  chooseHome: (provider) => tauriCore.invoke("choose_home", { provider }),
  getSettings: () => tauriCore.invoke("get_settings"),
  updateSettings: (settings) => tauriCore.invoke("update_settings", { settings }),
  syncTrayLanguage: (language) => tauriCore.invoke("sync_tray_language", { language }),
  checkUpdate: () => tauriCore.invoke("check_update"),
  installUpdate: () => tauriCore.invoke("install_update"),
  openExternal: (url) => tauriCore.invoke("open_external", { url }),
  startWindowDrag: async () => {
    const currentWindow = tauriWindow?.getCurrentWindow?.();
    if (currentWindow?.startDragging) {
      try {
        await currentWindow.startDragging();
        return;
      } catch {
        // Fall through to the local Rust command when the JS window API is unavailable or denied.
      }
    }
    return tauriCore.invoke("start_window_drag");
  }
};

document.body.dataset.platform = aiUsage.platform || "unknown";

let currentSettings = {
  activeProvider: "codex",
  enabledProviders: ["codex", "claude", "copilot", "cursor", "chatgpt"],
  codexHome: "",
  claudeHome: "",
  copilotHome: "",
  cursorHome: "",
  chatgptHome: "",
  language: "auto",
  theme: "system",
  accentColor: "blue",
  chartDays: DEFAULT_CHART_DAYS,
  autoRefreshEnabled: DEFAULT_AUTO_REFRESH_ENABLED,
  autoRefreshMinutes: DEFAULT_AUTO_REFRESH_MINUTES
};
let currentView = "home";
let currentHomeSection = "summary";
let currentChartMode = "tokens";
let activeSettingsSectionId = "settingsGeneralSection";
let lastStats = null;
let latestStatsRequestId = 0;
let currentLoading = false;
let updateInfo = { supported: false, available: false, currentVersion: null, version: null };
let updateInstalling = false;
let updateCheckInProgress = false;
let updateCheckError = "";
let rateLimitCountdownTimer = null;
let settingsSavedAt = new Date();
let autoRefreshTimer = null;
let autoRefreshCountdownTimer = null;
let autoRefreshDueAt = null;
let scanDiagnostics = [];
let cacheRebuildInProgress = false;
let cacheRebuildResult = null;
const statsCache = new Map();

function normalizeChartDays(value, fallback = DEFAULT_CHART_DAYS) {
  const days = Number(value);
  if (!Number.isFinite(days)) {
    return fallback;
  }
  return Math.min(MAX_CHART_DAYS, Math.max(MIN_CHART_DAYS, Math.round(days)));
}

function normalizeAutoRefreshMinutes(value, fallback = DEFAULT_AUTO_REFRESH_MINUTES) {
  const minutes = Number(value);
  if (!Number.isFinite(minutes)) {
    return fallback;
  }
  return Math.min(MAX_AUTO_REFRESH_MINUTES, Math.max(1, Math.round(minutes)));
}

function defaultCopilotHome() {
  if (aiUsage.platform === "windows" || aiUsage.platform === "win32") {
    return "%APPDATA%/Code/User/globalStorage/github.copilot-chat";
  }
  if (aiUsage.platform === "linux") {
    return "~/.config/Code/User/globalStorage/github.copilot-chat";
  }
  return "~/Library/Application Support/Code/User/globalStorage/github.copilot-chat";
}

function defaultCursorHome() {
  if (aiUsage.platform === "windows" || aiUsage.platform === "win32") {
    return "%APPDATA%/Cursor/User/globalStorage";
  }
  if (aiUsage.platform === "linux") {
    return "~/.config/Cursor/User/globalStorage";
  }
  return "~/Library/Application Support/Cursor/User/globalStorage";
}

function defaultChatgptHome() {
  if (aiUsage.platform === "windows" || aiUsage.platform === "win32") {
    return "%APPDATA%/com.openai.chat";
  }
  if (aiUsage.platform === "linux") {
    return "~/.config/com.openai.chat";
  }
  return "~/Library/Application Support/com.openai.chat";
}

const providerAssets = window.__AI_USAGE_ASSETS__ || {};
const PROVIDERS = {
  codex: {
    label: "Codex",
    initials: "CD",
    icon: providerAssets.codexLight || providerAssets.codex || "./assets/provider-codex-light.svg",
    darkIcon: providerAssets.codexDark || "./assets/provider-codex-dark.svg",
    defaultHome: "~/.codex"
  },
  claude: {
    label: "Claude Code",
    initials: "CC",
    icon: providerAssets.claude || "./assets/provider-claude.svg",
    defaultHome: "~/.claude"
  },
  copilot: {
    label: "GitHub Copilot",
    initials: "GH",
    icon: providerAssets.copilot || "./assets/provider-copilot.svg",
    defaultHome: defaultCopilotHome()
  },
  cursor: {
    label: "Cursor",
    initials: "CU",
    icon: providerAssets.cursor || "./assets/provider-cursor.svg",
    defaultHome: defaultCursorHome()
  },
  chatgpt: {
    label: "ChatGPT",
    initials: "CG",
    icon: providerAssets.chatgpt || "./assets/provider-chatgpt.svg",
    defaultHome: defaultChatgptHome()
  }
};
const PROVIDER_IDS = Object.keys(PROVIDERS);

function providerIconUrl(providerId, theme = document.body.dataset.theme) {
  const provider = PROVIDERS[providerId] || PROVIDERS.codex;
  return theme === "dark" && provider.darkIcon ? provider.darkIcon : provider.icon;
}

function setProviderImage(element, providerId) {
  element.dataset.providerLogo = providerId;
  element.src = providerIconUrl(providerId);
}

function syncProviderLogoTheme() {
  for (const image of document.querySelectorAll("img[data-provider-logo]")) {
    image.src = providerIconUrl(image.dataset.providerLogo);
  }
}

const I18N = {
  zh: {
    brandSubtitle: "用量监控",
    primaryNavigation: "主导航",
    overview: "概览",
    summary: "摘要",
    trend: "趋势",
    activityTab: "活动",
    switchDataSource: "⌘K 切换数据源",
    settings: "设置",
    settingsSearch: "搜索设置",
    settingsSearchResults: "搜索结果",
    settingsSearchDescription: "显示与“{query}”相关的设置。",
    settingsNoResults: "未找到相关设置",
    settingsGeneralTitle: "常规设置",
    settingsGeneralDescription: "配置应用的基本行为和使用偏好。",
    settingsAppearanceDescription: "调整界面主题与强调色。",
    settingsChartDescription: "设置摘要页图表默认展示的时间范围。",
    settingsDataDescription: "查看扫描缓存状态，并在需要时重建缓存。",
    settingsUpdateDescription: "查看版本信息并检查可用更新。",
    settingsProvidersDescription: "选择 AI 服务并管理其本地数据目录。",
    settingsAppGroup: "应用",
    settingsExperienceGroup: "使用体验",
    preferences: "偏好设置",
    backToApp: "返回应用",
    personal: "个人",
    providers: "AI 服务",
    general: "常规",
    appearance: "个性化",
    chart: "图表",
    dataAndCache: "数据与缓存",
    providerEnabled: "启用",
    repository: "仓库",
    feedback: "反馈",
    refresh: "刷新",
    usage: "用量",
    aiService: "AI 服务",
    remainingUsage: "剩余用量",
    quotaWindows: "额度窗口",
    officialQuota: "官方限额",
    quotaConsumption: "额度消耗",
    quotaSnapshot: "官方额度快照",
    localTokenActivity: "本地 token 活动",
    currentLimitsSafe: "当前两个额度窗口均安全",
    currentLimitSafe: "当前额度窗口状态安全",
    waitingLimitData: "等待新的额度快照",
    limitUnavailable: "暂未提供",
    estimatedRemainingAtReset: "预计重置时剩余 {percent}%",
    actualConsumption: "实际消耗",
    estimatedConsumption: "预计消耗",
    visibleSnapshotConsumption: "可见额度快照累计消耗 {percent}% · 当前官方已用 {official}%",
    localTokenSummary: "本地日志记录 {value} token",
    chartNoHistory: "只有最新额度快照；后续刷新后会逐步形成柱状历史。",
    chartHelp: "图表说明",
    chartHelpTitle: "如何阅读图表",
    chartHelpTokens: "每根柱子表示当天在本机日志中记录的 token 用量。",
    chartHelpFiveHour: "每根柱子表示 30 分钟内记录到的 5 小时额度消耗。",
    chartHelpWeekly: "每根柱子表示当天记录到的每周额度消耗。",
    chartHelpColor: "柱子越高、颜色越深，表示它相对当前图表内其他时段的数据量越大；虚线柱为预测值。",
    periodUsage: "周期用量",
    dataSource: "数据源",
    localEstimate: "本地日志估算",
    localActivityEstimate: "本地活动估算",
    localActivityEstimateHint: "基于本地会话活动推算，不代表 ChatGPT 官方限额。",
    recentActivity: "近期活动",
    activityCount: "{count} 次活动",
    rollingWindow: "滚动统计",
    localRecordsOnly: "仅统计本地记录",
    todayCost: "今日费用",
    periodCost: "近 {days} 天费用",
    costUnavailable: "暂无可靠定价",
    notAvailable: "不可用",
    todayTokens: "今日 token 用量",
    periodTokens: "近 {days} 天 token 用量",
    periodAccumulated: "{days} 天累计",
    periodTokenContext: "总 token {total} · 最近 {latest}",
    todayTokenUsageShare: "今日 token 占当前周期 token 的 {percent}%",
    threadsTotal: "累计会话",
    threadsTotalHint: "当前数据源本地目录中扫描到的全部会话记录，包括已归档会话，不受图表周期限制。",
    allLocalRecords: "全部本地记录",
    threadsActive: "活跃",
    tokensTotal: "总 token",
    updatedThisWeek: "近 7 天更新",
    activityTrend: "近 {days} 天趋势",
    usageInsights: "本期洞察",
    usageInsightsMeta: "基于本地记录",
    insightTrendLabel: "近 7 天 token 比前 7 天",
    insightTrendUp: "用量正在上升",
    insightTrendDown: "用量有所下降",
    insightTrendSteady: "用量基本稳定",
    insightTrendUnavailable: "需要至少 14 天记录",
    insightPeakLabel: "近 {days} 天用量最高日",
    insightPeakNote: "是日均的 {ratio} 倍",
    insightWorkspaceLabel: "最大工作区",
    insightWorkspaceBalanced: "累计用量分布较均衡",
    insightWorkspaceConcentrated: "累计用量相对集中",
    insightNoData: "暂无本地 token 记录",
    insightWorkspaceHint: "按当前数据源全部本地会话的累计 token 计算。",
    models: "模型",
    sources: "运行来源",
    workspaces: "工作区",
    recentThreads: "最近会话",
    settingsSavedAt: "已保存 · {time}",
    updateAvailable: "更新到 {version}",
    installingUpdate: "正在更新",
    installUpdateError: "无法安装更新",
    updates: "更新",
    checkForUpdates: "检查更新",
    checkingUpdate: "正在检查",
    noUpdateAvailable: "已是最新版本",
    updateUnsupported: "当前构建未启用自动更新",
    updateCheckFailed: "无法检查更新",
    updateAvailableStatus: "发现新版本 {version}",
    currentVersion: "当前版本",
    latestVersion: "最新版本",
    publishedAt: "发布时间",
    updateDialogEyebrow: "可用更新",
    updateDialogTitle: "发现新版本",
    updateDialogSubtitle: "安装完成后应用会自动重启。",
    releaseNotes: "更新说明",
    noReleaseNotes: "暂无更新说明。",
    installNow: "立即更新",
    later: "稍后",
    close: "关闭",
    enabledProviders: "启用 AI 服务",
    atLeastOneProvider: "至少启用一个 AI 服务",
    autoRefresh: "自动更新",
    autoRefreshEnabled: "启用",
    autoRefreshSuffix: "分钟",
    autoRefreshCountdown: "{time} 后刷新",
    scanProvider: "当前服务",
    latestScan: "最近扫描",
    scanCache: "扫描缓存",
    noScanDiagnostics: "刷新后显示",
    scanSummary: "{elapsed} ms · 命中 {hits}/{files}（{rate}%）",
    scanMeta: "重新解析 {parsed} · 删除 {deleted} · 失败 {failed}",
    scanCacheReady: "缓存运行正常",
    scanCompletedWithFailures: "扫描完成，{count} 个文件失败",
    cacheWriteSkipped: "扫描有 {count} 个失败文件，已跳过缓存写入",
    cacheWriteFailed: "扫描完成，但缓存写入失败",
    rebuildCache: "重建缓存",
    rebuildingCache: "正在重建",
    cacheRebuildComplete: "缓存已重建，统计已刷新",
    cacheRebuildError: "无法重建缓存",
    cacheRebuildUnverified: "统计已刷新，但无法确认缓存重建结果",
    dataFolder: "数据目录",
    chooseFolder: "选择目录",
    language: "语言",
    followSystem: "跟随系统",
    chinese: "中文",
    english: "English",
    theme: "主题",
    light: "浅色",
    dark: "深色",
    accentColor: "主题色",
    chartPeriod: "图表周期",
    chartPeriodPresets: "图表周期快捷选择",
    oneWeek: "一周",
    oneMonth: "一个月",
    threeMonths: "三个月",
    daysSuffix: "天",
    daysPeriod: "近 {days} 天",
    emptyData: "暂无数据",
    emptyRateLimits: "暂无剩余用量数据",
    emptyRecentThreads: "暂无最近会话",
    updatedAt: "更新于 {date}",
    updatesIn: "更新 {time}",
    updatingSoon: "即将更新",
    usageEstimated: "按本地日志估算",
    usedUsage: "{percent}% 已使用",
    usageHeadroom: "余量 {percent}%",
    usageOverrun: "超额 {percent}%",
    lastsUntilReset: "持续到重置",
    projectedEmpty: "预计 {time} 后耗尽",
    idealUsageMarker: "理论使用 {percent}%",
    resetAt: "{label} · {time} 重置",
    waitingForLogs: "等待 {provider} 日志",
    readStatsError: "无法读取 {provider} 统计",
    switchHomeError: "无法切换 {provider} 目录",
    tokens: "tokens",
    subtask: "子任务",
    unknown: "Unknown",
    untitled: "Untitled"
  },
  en: {
    brandSubtitle: "Usage monitor",
    primaryNavigation: "Primary navigation",
    overview: "Overview",
    summary: "Summary",
    trend: "Trend",
    activityTab: "Activity",
    switchDataSource: "⌘K Switch source",
    settings: "Settings",
    settingsSearch: "Search settings",
    settingsSearchResults: "Search results",
    settingsSearchDescription: "Settings related to “{query}”.",
    settingsNoResults: "No matching settings",
    settingsGeneralTitle: "General settings",
    settingsGeneralDescription: "Configure the app's core behavior and preferences.",
    settingsAppearanceDescription: "Adjust the interface theme and accent color.",
    settingsChartDescription: "Set the default time range for overview charts.",
    settingsDataDescription: "Review scan cache status and rebuild it when needed.",
    settingsUpdateDescription: "Review version information and check for updates.",
    settingsProvidersDescription: "Choose AI services and manage their local data folders.",
    settingsAppGroup: "App",
    settingsExperienceGroup: "Experience",
    preferences: "Preferences",
    backToApp: "Back to app",
    personal: "Personal",
    providers: "AI services",
    general: "General",
    appearance: "Personalization",
    chart: "Chart",
    dataAndCache: "Data & cache",
    providerEnabled: "Enabled",
    repository: "GitHub",
    feedback: "Feedback",
    refresh: "Refresh",
    usage: "usage",
    aiService: "AI service",
    remainingUsage: "Remaining usage",
    quotaWindows: "Quota windows",
    officialQuota: "Official quota",
    quotaConsumption: "Quota consumption",
    quotaSnapshot: "Official quota snapshots",
    localTokenActivity: "Local token activity",
    currentLimitsSafe: "Both quota windows are in a safe range",
    currentLimitSafe: "The current quota window is in a safe range",
    waitingLimitData: "Waiting for a new quota snapshot",
    limitUnavailable: "Not currently available",
    estimatedRemainingAtReset: "Estimated {percent}% remaining at reset",
    actualConsumption: "Actual consumption",
    estimatedConsumption: "Estimated consumption",
    visibleSnapshotConsumption: "Visible snapshots used {percent}% · official usage is {official}%",
    localTokenSummary: "Local logs recorded {value} tokens",
    chartNoHistory: "Only the latest quota snapshot is available; bars will accumulate after future refreshes.",
    chartHelp: "Chart help",
    chartHelpTitle: "How to read this chart",
    chartHelpTokens: "Each bar shows token usage recorded in local logs for that day.",
    chartHelpFiveHour: "Each bar shows 5-hour quota consumption recorded within a 30-minute interval.",
    chartHelpWeekly: "Each bar shows weekly quota consumption recorded for that day.",
    chartHelpColor: "Taller, darker bars contain more data relative to the other periods in this chart; dashed bars are projections.",
    periodUsage: "Period usage",
    dataSource: "Data source",
    localEstimate: "Local log estimate",
    localActivityEstimate: "Local activity estimate",
    localActivityEstimateHint: "Estimated from local conversation activity; it is not an official ChatGPT quota.",
    recentActivity: "Recent activity",
    activityCount: "{count} activities",
    rollingWindow: "Rolling window",
    localRecordsOnly: "Local records only",
    todayCost: "Today cost",
    periodCost: "{days}-day cost",
    costUnavailable: "Reliable pricing unavailable",
    notAvailable: "N/A",
    todayTokens: "Today token usage",
    periodTokens: "{days}-day token usage",
    periodAccumulated: "{days}-day total",
    periodTokenContext: "Total tokens {total} · latest {latest}",
    todayTokenUsageShare: "Today's token usage is {percent}% of the current period",
    threadsTotal: "All-time sessions",
    threadsTotalHint: "All session records scanned from the current provider's local data folder, including archived sessions and regardless of the chart period.",
    allLocalRecords: "All local records",
    threadsActive: "Active",
    tokensTotal: "Total tokens",
    updatedThisWeek: "Updated in 7 days",
    activityTrend: "{days}-day trend",
    usageInsights: "Period insights",
    usageInsightsMeta: "Based on local records",
    insightTrendLabel: "Last 7 days vs previous 7 days",
    insightTrendUp: "Usage is rising",
    insightTrendDown: "Usage is declining",
    insightTrendSteady: "Usage is steady",
    insightTrendUnavailable: "At least 14 days of records required",
    insightPeakLabel: "Peak day in {days} days",
    insightPeakNote: "{ratio}× the daily average",
    insightWorkspaceLabel: "Top workspace",
    insightWorkspaceBalanced: "Cumulative usage is fairly distributed",
    insightWorkspaceConcentrated: "Cumulative usage is concentrated",
    insightNoData: "No local token records",
    insightWorkspaceHint: "Calculated from cumulative tokens across all local sessions in the current data source.",
    models: "Models",
    sources: "Sources",
    workspaces: "Workspaces",
    recentThreads: "Recent threads",
    settingsSavedAt: "Saved · {time}",
    updateAvailable: "Update to {version}",
    installingUpdate: "Updating",
    installUpdateError: "Unable to install update",
    updates: "Updates",
    checkForUpdates: "Check for updates",
    checkingUpdate: "Checking",
    noUpdateAvailable: "You are up to date",
    updateUnsupported: "Automatic updates are not enabled in this build",
    updateCheckFailed: "Unable to check for updates",
    updateAvailableStatus: "Version {version} is available",
    currentVersion: "Current version",
    latestVersion: "Latest version",
    publishedAt: "Published",
    updateDialogEyebrow: "Update available",
    updateDialogTitle: "A new version is available",
    updateDialogSubtitle: "The app will restart after installation.",
    releaseNotes: "Release notes",
    noReleaseNotes: "No release notes available.",
    installNow: "Update now",
    later: "Later",
    close: "Close",
    enabledProviders: "Enabled AI services",
    atLeastOneProvider: "Keep at least one AI service enabled",
    autoRefresh: "Auto refresh",
    autoRefreshEnabled: "Enabled",
    autoRefreshSuffix: "min",
    autoRefreshCountdown: "Refresh in {time}",
    scanProvider: "Current service",
    latestScan: "Latest scan",
    scanCache: "Scan cache",
    noScanDiagnostics: "Shown after refresh",
    scanSummary: "{elapsed} ms · {hits}/{files} cached ({rate}%)",
    scanMeta: "{parsed} parsed · {deleted} deleted · {failed} failed",
    scanCacheReady: "Cache is working",
    scanCompletedWithFailures: "Scan completed with {count} failed files",
    cacheWriteSkipped: "{count} files failed to scan; cache write was skipped",
    cacheWriteFailed: "Scan completed, but the cache could not be written",
    rebuildCache: "Rebuild cache",
    rebuildingCache: "Rebuilding",
    cacheRebuildComplete: "Cache rebuilt and stats refreshed",
    cacheRebuildError: "Unable to rebuild cache",
    cacheRebuildUnverified: "Stats refreshed, but the cache rebuild result could not be verified",
    dataFolder: "Data folder",
    chooseFolder: "Choose folder",
    language: "Language",
    followSystem: "Follow system",
    chinese: "中文",
    english: "English",
    theme: "Theme",
    light: "Light",
    dark: "Dark",
    accentColor: "Accent color",
    chartPeriod: "Chart period",
    chartPeriodPresets: "Chart period presets",
    oneWeek: "1 week",
    oneMonth: "1 month",
    threeMonths: "3 months",
    daysSuffix: "days",
    daysPeriod: "Last {days} days",
    emptyData: "No data",
    emptyRateLimits: "No remaining usage data",
    emptyRecentThreads: "No recent threads",
    updatedAt: "Updated {date}",
    updatesIn: "Updates in {time}",
    updatingSoon: "Updating soon",
    usageEstimated: "Estimated from local logs",
    usedUsage: "{percent}% used",
    usageHeadroom: "{percent}% headroom",
    usageOverrun: "{percent}% over pace",
    lastsUntilReset: "Lasts until reset",
    projectedEmpty: "Empty in {time}",
    idealUsageMarker: "Ideal usage {percent}%",
    resetAt: "{label} · resets {time}",
    waitingForLogs: "Waiting for {provider} logs",
    readStatsError: "Unable to read {provider} stats",
    switchHomeError: "Unable to switch {provider} folder",
    tokens: "tokens",
    subtask: "Subtask",
    unknown: "Unknown",
    untitled: "Untitled"
  }
};

const elements = {
  documentTitle: document.querySelector("title"),
  brandSubtitle: document.getElementById("brandSubtitle"),
  primaryNav: document.getElementById("primaryNav"),
  homeView: document.getElementById("homeView"),
  settingsView: document.getElementById("settingsView"),
  homeButton: document.getElementById("homeButton"),
  trendButton: document.getElementById("trendButton"),
  activityButton: document.getElementById("activityButton"),
  settingsButton: document.getElementById("settingsButton"),
  sourceShortcutButton: document.getElementById("sourceShortcutButton"),
  providerMenuButton: document.getElementById("providerMenuButton"),
  headerProviderLogo: document.getElementById("headerProviderLogo"),
  headerProviderLabel: document.getElementById("headerProviderLabel"),
  settingsBackButton: document.getElementById("settingsBackButton"),
  settingsBackLabel: document.getElementById("settingsBackLabel"),
  settingsWindowTitle: document.getElementById("settingsWindowTitle"),
  settingsSearchInput: document.getElementById("settingsSearchInput"),
  settingsProvidersTab: document.getElementById("settingsProvidersTab"),
  settingsProviderTabs: document.getElementById("settingsProviderTabs"),
  settingsSectionDescription: document.getElementById("settingsSectionDescription"),
  settingsNoResults: document.getElementById("settingsNoResults"),
  settingsNoResultsTitle: document.getElementById("settingsNoResultsTitle"),
  settingsPersonalLabel: document.getElementById("settingsPersonalLabel"),
  settingsProvidersLabel: document.getElementById("settingsProvidersLabel"),
  settingsGeneralNavLabel: document.getElementById("settingsGeneralNavLabel"),
  settingsAppearanceNavLabel: document.getElementById("settingsAppearanceNavLabel"),
  settingsChartNavLabel: document.getElementById("settingsChartNavLabel"),
  settingsDataNavLabel: document.getElementById("settingsDataNavLabel"),
  settingsUpdateNavLabel: document.getElementById("settingsUpdateNavLabel"),
  settingsCodexNavLabel: document.getElementById("settingsCodexNavLabel"),
  settingsClaudeNavLabel: document.getElementById("settingsClaudeNavLabel"),
  settingsCopilotNavLabel: document.getElementById("settingsCopilotNavLabel"),
  settingsCursorNavLabel: document.getElementById("settingsCursorNavLabel"),
  settingsChatgptNavLabel: document.getElementById("settingsChatgptNavLabel"),
  repositoryLink: document.getElementById("repositoryLink"),
  issueLink: document.getElementById("issueLink"),
  sidebarProviderSection: document.getElementById("sidebarProviderSection"),
  sidebarProviderLabel: document.getElementById("sidebarProviderLabel"),
  providerOptions: document.getElementById("providerOptions"),
  sidebarUsageSection: document.getElementById("sidebarUsageSection"),
  sidebarUsageLabel: document.getElementById("sidebarUsageLabel"),
  overviewSourceLabel: document.getElementById("overviewSourceLabel"),
  overviewEstimateLabel: document.getElementById("overviewEstimateLabel"),
  overviewPeriod: document.getElementById("overviewPeriod"),
  overviewPeriodLabel: document.getElementById("overviewPeriodLabel"),
  overviewProviderLogo: document.getElementById("overviewProviderLogo"),
  overviewAccountName: document.getElementById("overviewAccountName"),
  overviewAccountPlan: document.getElementById("overviewAccountPlan"),
  sidebarRemainingUsage: document.getElementById("sidebarRemainingUsage"),
  sidebarPeriodMeta: document.getElementById("sidebarPeriodMeta"),
  viewEyebrow: document.getElementById("viewEyebrow"),
  viewTitle: document.getElementById("viewTitle"),
  overviewProvider: document.getElementById("overviewProvider"),
  accountInitials: document.getElementById("accountInitials"),
  accountName: document.getElementById("accountName"),
  accountPlan: document.getElementById("accountPlan"),
  errorPanel: document.getElementById("errorPanel"),
  todayCost: document.getElementById("todayCost"),
  periodCost: document.getElementById("periodCost"),
  periodUsageMeta: document.getElementById("periodUsageMeta"),
  periodTokens: document.getElementById("periodTokens"),
  periodTokensContext: document.getElementById("periodTokensContext"),
  todayTokens: document.getElementById("todayTokens"),
  todayTokensMeter: document.getElementById("todayTokensMeter"),
  todayCostLabel: document.getElementById("todayCostLabel"),
  periodCostLabel: document.getElementById("periodCostLabel"),
  periodTokensLabel: document.getElementById("periodTokensLabel"),
  todayTokensLabel: document.getElementById("todayTokensLabel"),
  activityTitle: document.getElementById("activityTitle"),
  threadsTotalLabel: document.getElementById("threadsTotalLabel"),
  threadsTotalMeta: document.getElementById("threadsTotalMeta"),
  threadsActiveLabel: document.getElementById("threadsActiveLabel"),
  tokensTotalLabel: document.getElementById("tokensTotalLabel"),
  updatedThisWeekLabel: document.getElementById("updatedThisWeekLabel"),
  threadsTotal: document.getElementById("threadsTotal"),
  threadsActive: document.getElementById("threadsActive"),
  tokensTotal: document.getElementById("tokensTotal"),
  updatedThisWeek: document.getElementById("updatedThisWeek"),
  lastUpdated: document.getElementById("lastUpdated"),
  rateLimitUpdated: document.getElementById("rateLimitUpdated"),
  rateLimitTitle: document.getElementById("rateLimitTitle"),
  statusHeading: document.getElementById("statusHeading"),
  quotaSourceBadge: document.getElementById("quotaSourceBadge"),
  chartSourceLabel: document.getElementById("chartSourceLabel"),
  chartHelpButton: document.getElementById("chartHelpButton"),
  chartHelpPopover: document.getElementById("chartHelpPopover"),
  chartHelpTitle: document.getElementById("chartHelpTitle"),
  chartHelpText: document.getElementById("chartHelpText"),
  chartLegend: document.getElementById("chartLegend"),
  chartInterpretation: document.getElementById("chartInterpretation"),
  chartModeButtons: Array.from(document.querySelectorAll("[data-chart-mode]")),
  usageInsightsTitle: document.getElementById("usageInsightsTitle"),
  usageInsightsMeta: document.getElementById("usageInsightsMeta"),
  insightTrendLabel: document.getElementById("insightTrendLabel"),
  insightTrendValue: document.getElementById("insightTrendValue"),
  insightTrendNote: document.getElementById("insightTrendNote"),
  insightSparkline: document.getElementById("insightSparkline"),
  insightPeakLabel: document.getElementById("insightPeakLabel"),
  insightPeakValue: document.getElementById("insightPeakValue"),
  insightPeakNote: document.getElementById("insightPeakNote"),
  insightWorkspaceLabel: document.getElementById("insightWorkspaceLabel"),
  insightWorkspaceValue: document.getElementById("insightWorkspaceValue"),
  insightWorkspaceNote: document.getElementById("insightWorkspaceNote"),
  activityMeta: document.getElementById("activityMeta"),
  modelTitle: document.getElementById("modelTitle"),
  sourceTitle: document.getElementById("sourceTitle"),
  workspaceTitle: document.getElementById("workspaceTitle"),
  recentThreadsTitle: document.getElementById("recentThreadsTitle"),
  dailyChart: document.getElementById("dailyChart"),
  chartTooltip: document.getElementById("chartTooltip"),
  rateLimitList: document.getElementById("rateLimitList"),
  modelList: document.getElementById("modelList"),
  sourceList: document.getElementById("sourceList"),
  workspaceList: document.getElementById("workspaceList"),
  recentThreads: document.getElementById("recentThreads"),
  updateButton: document.getElementById("updateButton"),
  refreshButton: document.getElementById("refreshButton"),
  autoRefreshCountdown: document.getElementById("autoRefreshCountdown"),
  chooseCodexHomeButton: document.getElementById("chooseCodexHomeButton"),
  chooseClaudeHomeButton: document.getElementById("chooseClaudeHomeButton"),
  chooseCopilotHomeButton: document.getElementById("chooseCopilotHomeButton"),
  chooseCursorHomeButton: document.getElementById("chooseCursorHomeButton"),
  chooseChatgptHomeButton: document.getElementById("chooseChatgptHomeButton"),
  codexHomeValue: document.getElementById("codexHomeValue"),
  claudeHomeValue: document.getElementById("claudeHomeValue"),
  copilotHomeValue: document.getElementById("copilotHomeValue"),
  cursorHomeValue: document.getElementById("cursorHomeValue"),
  chatgptHomeValue: document.getElementById("chatgptHomeValue"),
  settingsPanelTitle: document.getElementById("settingsPanelTitle"),
  settingsGeneralTitle: document.getElementById("settingsGeneralTitle"),
  settingsProvidersContentTitle: document.getElementById("settingsProvidersContentTitle"),
  settingsCodexProviderTitle: document.getElementById("settingsCodexProviderTitle"),
  settingsClaudeProviderTitle: document.getElementById("settingsClaudeProviderTitle"),
  settingsCopilotProviderTitle: document.getElementById("settingsCopilotProviderTitle"),
  settingsCursorProviderTitle: document.getElementById("settingsCursorProviderTitle"),
  settingsChatgptProviderTitle: document.getElementById("settingsChatgptProviderTitle"),
  settingsAppearanceTitle: document.getElementById("settingsAppearanceTitle"),
  settingsChartTitle: document.getElementById("settingsChartTitle"),
  settingsDataTitle: document.getElementById("settingsDataTitle"),
  settingsUpdateTitle: document.getElementById("settingsUpdateTitle"),
  scanProviderLabel: document.getElementById("scanProviderLabel"),
  scanProviderValue: document.getElementById("scanProviderValue"),
  latestScanLabel: document.getElementById("latestScanLabel"),
  scanDiagnosticsSummary: document.getElementById("scanDiagnosticsSummary"),
  scanDiagnosticsMeta: document.getElementById("scanDiagnosticsMeta"),
  cacheManagementLabel: document.getElementById("cacheManagementLabel"),
  rebuildCacheStatus: document.getElementById("rebuildCacheStatus"),
  rebuildCacheButton: document.getElementById("rebuildCacheButton"),
  providerEnabledLabels: Array.from(document.querySelectorAll("[data-provider-enabled-label]")),
  providerToggleLabels: Array.from(document.querySelectorAll("[data-provider-toggle-label]")),
  codexHomeLabel: document.getElementById("codexHomeLabel"),
  claudeHomeLabel: document.getElementById("claudeHomeLabel"),
  copilotHomeLabel: document.getElementById("copilotHomeLabel"),
  cursorHomeLabel: document.getElementById("cursorHomeLabel"),
  chatgptHomeLabel: document.getElementById("chatgptHomeLabel"),
  languageLabel: document.getElementById("languageLabel"),
  languageSelect: document.getElementById("languageSelect"),
  languageAutoOption: document.getElementById("languageAutoOption"),
  languageZhOption: document.getElementById("languageZhOption"),
  languageEnOption: document.getElementById("languageEnOption"),
  autoRefreshLabel: document.getElementById("autoRefreshLabel"),
  autoRefreshEnabledInput: document.getElementById("autoRefreshEnabledInput"),
  autoRefreshEnabledLabel: document.getElementById("autoRefreshEnabledLabel"),
  autoRefreshInput: document.getElementById("autoRefreshInput"),
  autoRefreshSuffix: document.getElementById("autoRefreshSuffix"),
  themeLabel: document.getElementById("themeLabel"),
  themeSystemOption: document.getElementById("themeSystemOption"),
  themeLightOption: document.getElementById("themeLightOption"),
  themeDarkOption: document.getElementById("themeDarkOption"),
  accentLabel: document.getElementById("accentLabel"),
  accentOptions: document.getElementById("accentOptions"),
  chartPeriodLabel: document.getElementById("chartPeriodLabel"),
  periodPresets: document.getElementById("periodPresets"),
  period7Button: document.getElementById("period7Button"),
  period30Button: document.getElementById("period30Button"),
  period90Button: document.getElementById("period90Button"),
  daysSuffix: document.getElementById("daysSuffix"),
  currentVersionLabel: document.getElementById("currentVersionLabel"),
  currentVersionValue: document.getElementById("currentVersionValue"),
  latestVersionLabel: document.getElementById("latestVersionLabel"),
  latestVersionValue: document.getElementById("latestVersionValue"),
  settingsUpdateStatus: document.getElementById("settingsUpdateStatus"),
  checkUpdateButton: document.getElementById("checkUpdateButton"),
  settingsInstallUpdateButton: document.getElementById("settingsInstallUpdateButton"),
  updateDialog: document.getElementById("updateDialog"),
  updateDialogEyebrow: document.getElementById("updateDialogEyebrow"),
  updateDialogTitle: document.getElementById("updateDialogTitle"),
  updateDialogSubtitle: document.getElementById("updateDialogSubtitle"),
  closeUpdateDialogButton: document.getElementById("closeUpdateDialogButton"),
  dialogCurrentVersionLabel: document.getElementById("dialogCurrentVersionLabel"),
  dialogCurrentVersion: document.getElementById("dialogCurrentVersion"),
  dialogLatestVersionLabel: document.getElementById("dialogLatestVersionLabel"),
  dialogLatestVersion: document.getElementById("dialogLatestVersion"),
  dialogPublishedAtLabel: document.getElementById("dialogPublishedAtLabel"),
  dialogPublishedAt: document.getElementById("dialogPublishedAt"),
  updateNotesTitle: document.getElementById("updateNotesTitle"),
  updateNotes: document.getElementById("updateNotes"),
  postponeUpdateButton: document.getElementById("postponeUpdateButton"),
  installUpdateButton: document.getElementById("installUpdateButton"),
  providerButtons: Array.from(document.querySelectorAll(".sidebar-provider [data-provider]")),
  settingsProviderNavButtons: Array.from(document.querySelectorAll("[data-settings-provider]")),
  enabledProviderInputs: Array.from(document.querySelectorAll("[data-enabled-provider]")),
  settingsNavButtons: Array.from(document.querySelectorAll("[data-settings-section]")),
  settingsSections: Array.from(document.querySelectorAll(".settings-list > .settings-section")),
  themeSelect: document.getElementById("themeSelect"),
  accentButtons: Array.from(document.querySelectorAll("[data-accent]")),
  periodButtons: Array.from(document.querySelectorAll("[data-days]")),
  chartDaysInput: document.getElementById("chartDaysInput"),
  settingsStatus: document.getElementById("settingsStatus"),
  settingsContent: document.querySelector(".settings-content")
};

function systemLanguage() {
  const language = [navigator.language, ...(navigator.languages || [])].filter(Boolean).find(Boolean) || "en";
  return language.toLowerCase().startsWith("zh") ? "zh" : "en";
}

function currentLanguage() {
  return currentSettings.language === "zh" || currentSettings.language === "en"
    ? currentSettings.language
    : systemLanguage();
}

function enabledProviders(settings = currentSettings) {
  const enabled = Array.isArray(settings.enabledProviders) ? settings.enabledProviders : PROVIDER_IDS;
  const normalized = enabled.filter((provider) => PROVIDER_IDS.includes(provider));
  return normalized.length ? normalized : [...PROVIDER_IDS];
}

function isProviderEnabled(providerId, settings = currentSettings) {
  return enabledProviders(settings).includes(providerId);
}

function firstEnabledProvider(settings = currentSettings) {
  return enabledProviders(settings)[0] || "codex";
}

function applyProviderEstimateText(providerId = currentSettings.activeProvider) {
  const isChatgpt = providerId === "chatgpt";
  elements.overviewEstimateLabel.textContent = isChatgpt ? t("localActivityEstimate") : t("localEstimate");
  elements.overviewEstimateLabel.setAttribute(
    "title",
    isChatgpt ? t("localActivityEstimateHint") : t("localEstimate")
  );
  elements.rateLimitTitle.textContent = isChatgpt ? t("recentActivity") : t("quotaWindows");
  elements.rateLimitTitle.setAttribute("title", isChatgpt ? t("localActivityEstimateHint") : t("quotaWindows"));
}

function localeForLanguage() {
  return currentLanguage() === "zh" ? "zh-CN" : "en-US";
}

function t(key, values = {}) {
  const dictionary = I18N[currentLanguage()] || I18N.en;
  const template = dictionary[key] || I18N.en[key] || key;
  return template.replace(/\{(\w+)\}/g, (_, name) => values[name] ?? "");
}

function formatNumber(value) {
  return new Intl.NumberFormat(localeForLanguage(), {
    maximumFractionDigits: 0
  }).format(value || 0);
}

function formatCompact(value) {
  const number = Math.abs(Number(value) || 0);
  const sign = Number(value) < 0 ? "-" : "";
  const units = [
    { suffix: "B", value: 1_000_000_000 },
    { suffix: "M", value: 1_000_000 },
    { suffix: "K", value: 1_000 }
  ];
  const unit = units.find((candidate) => number >= candidate.value);

  if (!unit) {
    return `${sign}${formatNumber(number)}`;
  }

  const scaled = number / unit.value;
  const digits = scaled >= 100 ? 0 : scaled >= 10 ? 1 : 1;
  return `${sign}${scaled.toFixed(digits).replace(/\.0$/, "")}${unit.suffix}`;
}

function formatCurrency(value) {
  return `$${new Intl.NumberFormat(localeForLanguage(), {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2
  }).format(value || 0)}`;
}

function formatPercent(value) {
  return `${Math.round(Number(value) || 0)}%`;
}

function clampPercent(value) {
  return Math.max(0, Math.min(100, Number(value) || 0));
}

function formatResetTime(limit) {
  if (!limit?.resetsAt) return "-";
  const resetDate = new Date(limit.resetsAt);
  if (Number.isNaN(resetDate.getTime())) return "-";

  if (Number(limit.windowMinutes) <= 24 * 60) {
    return new Intl.DateTimeFormat(localeForLanguage(), {
      hour: "2-digit",
      minute: "2-digit"
    }).format(resetDate);
  }

  return new Intl.DateTimeFormat(localeForLanguage(), {
    month: "short",
    day: "numeric"
  }).format(resetDate);
}

function formatCountdown(value) {
  if (!value) return "-";
  const resetDate = new Date(value);
  if (Number.isNaN(resetDate.getTime())) return "-";

  let remainingSeconds = Math.max(0, Math.ceil((resetDate.getTime() - Date.now()) / 1000));
  if (remainingSeconds <= 0) return t("updatingSoon");

  const days = Math.floor(remainingSeconds / 86400);
  remainingSeconds -= days * 86400;
  const hours = Math.floor(remainingSeconds / 3600);
  remainingSeconds -= hours * 3600;
  const minutes = Math.floor(remainingSeconds / 60);
  const seconds = remainingSeconds - minutes * 60;

  if (currentLanguage() === "zh") {
    if (days > 0) return `${days}天 ${hours}小时`;
    if (hours > 0) return `${hours}小时 ${minutes}分`;
    if (minutes > 0) return `${minutes}分 ${seconds}秒`;
    return `${seconds}秒`;
  }

  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${seconds}s`;
  return `${seconds}s`;
}

function formatDuration(milliseconds) {
  if (!Number.isFinite(milliseconds) || milliseconds <= 0) {
    return t("updatingSoon");
  }
  return formatCountdown(new Date(Date.now() + milliseconds).toISOString());
}

function formatUpdateCountdown(value) {
  const countdown = formatCountdown(value);
  return countdown === "-" ? "-" : t("updatesIn", { time: countdown });
}

function rateLimitPace(limit) {
  const usedPercent = clampPercent(limit?.usedPercent);
  const remainingPercent = clampPercent(limit?.remainingPercent);
  const resetDate = new Date(limit?.resetsAt || "");
  const resetMs = resetDate.getTime();
  const windowMs = Number(limit?.windowMinutes || 0) * 60 * 1000;

  if (!Number.isFinite(resetMs) || windowMs <= 0) {
    return {
      usedPercent,
      remainingPercent,
      idealPercent: null,
      balancePercent: null,
      exhaustionMs: null,
      projectedRemainingPercent: null
    };
  }

  const nowMs = Date.now();
  const startMs = resetMs - windowMs;
  const elapsedMs = Math.max(0, Math.min(windowMs, nowMs - startMs));
  const idealPercent = clampPercent((elapsedMs / windowMs) * 100);
  const balancePercent = idealPercent - usedPercent;
  let exhaustionMs = null;
  let projectedRemainingPercent = null;

  if (usedPercent >= 100) {
    exhaustionMs = 0;
  } else if (usedPercent > 0 && elapsedMs > 0) {
    const usedPercentPerMs = usedPercent / elapsedMs;
    const projectedMs = remainingPercent / usedPercentPerMs;
    projectedRemainingPercent = clampPercent(100 - usedPercentPerMs * windowMs);
    if (Number.isFinite(projectedMs) && nowMs + projectedMs < resetMs) {
      exhaustionMs = Math.max(0, projectedMs);
    }
  }

  return {
    usedPercent,
    remainingPercent,
    idealPercent,
    balancePercent,
    exhaustionMs,
    projectedRemainingPercent
  };
}

function updateRateLimitCountdowns() {
  const nodes = document.querySelectorAll("[data-reset-countdown]");
  for (const node of nodes) {
    node.textContent = formatUpdateCountdown(node.dataset.resetCountdown);
  }
  const exhaustionNodes = document.querySelectorAll("[data-exhaustion-countdown]");
  for (const node of exhaustionNodes) {
    node.textContent = t("projectedEmpty", { time: formatCountdown(node.dataset.exhaustionCountdown) });
  }
  if (currentView !== "home" || (nodes.length === 0 && exhaustionNodes.length === 0)) {
    stopRateLimitCountdownTimer();
  }
}

function startRateLimitCountdownTimer() {
  if (rateLimitCountdownTimer !== null) return;
  rateLimitCountdownTimer = window.setInterval(updateRateLimitCountdowns, 1000);
}

function stopRateLimitCountdownTimer() {
  if (rateLimitCountdownTimer === null) return;
  window.clearInterval(rateLimitCountdownTimer);
  rateLimitCountdownTimer = null;
}

function syncRateLimitCountdownTimer() {
  if (currentView === "home" && document.querySelector("[data-reset-countdown], [data-exhaustion-countdown]")) {
    startRateLimitCountdownTimer();
  } else {
    stopRateLimitCountdownTimer();
  }
}

function stopAutoRefreshTimer() {
  if (autoRefreshTimer !== null) {
    window.clearTimeout(autoRefreshTimer);
  }
  autoRefreshTimer = null;
  autoRefreshDueAt = null;
  stopAutoRefreshCountdownTimer();
  renderAutoRefreshCountdown();
}

function stopAutoRefreshCountdownTimer() {
  if (autoRefreshCountdownTimer === null) return;
  window.clearInterval(autoRefreshCountdownTimer);
  autoRefreshCountdownTimer = null;
}

function renderAutoRefreshCountdown() {
  const shouldShow = Boolean(currentView === "home" && currentSettings.autoRefreshEnabled && autoRefreshDueAt);
  elements.autoRefreshCountdown.hidden = !shouldShow;
  if (!shouldShow) return;

  const text = t("autoRefreshCountdown", { time: formatCountdown(autoRefreshDueAt) });
  elements.autoRefreshCountdown.textContent = text;
  elements.autoRefreshCountdown.setAttribute("title", text);
}

function startAutoRefreshCountdownTimer() {
  stopAutoRefreshCountdownTimer();
  renderAutoRefreshCountdown();
  if (!autoRefreshDueAt || !currentSettings.autoRefreshEnabled) return;
  autoRefreshCountdownTimer = window.setInterval(renderAutoRefreshCountdown, 1000);
}

function autoRefreshDelayMs() {
  if (!currentSettings.autoRefreshEnabled) {
    return 0;
  }
  const minutes = normalizeAutoRefreshMinutes(currentSettings.autoRefreshMinutes);
  return minutes * 60 * 1000;
}

function scheduleAutoRefreshTimer() {
  stopAutoRefreshTimer();
  const delay = autoRefreshDelayMs();
  if (delay <= 0) return;
  autoRefreshDueAt = new Date(Date.now() + delay);
  startAutoRefreshCountdownTimer();
  autoRefreshTimer = window.setTimeout(async () => {
    autoRefreshTimer = null;
    autoRefreshDueAt = null;
    stopAutoRefreshCountdownTimer();
    renderAutoRefreshCountdown();
    if (currentLoading) {
      scheduleAutoRefreshTimer();
      return;
    }
    await refreshStats({ force: true, showLoading: false });
    scheduleAutoRefreshTimer();
  }, delay);
}

function resetAutoRefreshTimer() {
  scheduleAutoRefreshTimer();
}

function availableRateLimitWindows(rateLimits) {
  if (!Array.isArray(rateLimits?.windows)) return [];
  return rateLimits.windows.filter((window) => {
    const usedPercent = Number(window?.usedPercent);
    const remainingPercent = Number(window?.remainingPercent);
    const windowMinutes = Number(window?.windowMinutes);
    return (
      Number.isFinite(usedPercent) &&
      Number.isFinite(remainingPercent) &&
      Number.isFinite(windowMinutes) &&
      windowMinutes > 0
    );
  });
}

function primaryRateLimit(stats) {
  return availableRateLimitWindows(stats.rateLimits)[0] || null;
}

function availableActivityWindows(activity) {
  if (!Array.isArray(activity?.windows)) return [];
  return activity.windows.filter((window) => {
    const count = Number(window?.count);
    const windowMinutes = Number(window?.windowMinutes);
    return Number.isFinite(count) && count >= 0 && Number.isFinite(windowMinutes) && windowMinutes > 0;
  });
}

function primaryActivityWindow(stats) {
  return availableActivityWindows(stats.activity)[0] || null;
}

function formatLimitLabel(limit) {
  const minutes = Number(limit?.windowMinutes) || 0;
  if (minutes === 300) return currentLanguage() === "zh" ? "5 小时" : "5 hours";
  if (minutes === 10080) return currentLanguage() === "zh" ? "每周" : "Weekly";
  if (minutes >= 10080 && minutes % 10080 === 0) {
    const weeks = minutes / 10080;
    return currentLanguage() === "zh" ? `${weeks} 周` : `${weeks} weeks`;
  }
  if (minutes >= 1440 && minutes % 1440 === 0) {
    const days = minutes / 1440;
    return currentLanguage() === "zh" ? `${days} 天` : `${days} days`;
  }
  if (minutes >= 60 && minutes % 60 === 0) {
    const hours = minutes / 60;
    return currentLanguage() === "zh" ? `${hours} 小时` : `${hours} hours`;
  }
  return currentLanguage() === "zh" ? `${minutes} 分钟` : `${minutes} minutes`;
}

function formatLimitMeta(limit) {
  const label = formatLimitLabel(limit);
  return limit?.resetsAt ? t("resetAt", { label, time: formatResetTime(limit) }) : label;
}

function displaySourceName(name) {
  if (name === "子任务" || name === "Subtask") return t("subtask");
  if (!name || name === "Unknown") return t("unknown");
  return name;
}

function statsRatioPercent(value, total) {
  const numerator = Number(value);
  const denominator = Number(total);
  if (!Number.isFinite(numerator) || !Number.isFinite(denominator) || denominator <= 0) {
    return 0;
  }
  return (numerator / denominator) * 100;
}

function parseDateValue(value) {
  if (!value) return null;
  const date = new Date(value);
  if (!Number.isNaN(date.getTime())) return date;
  if (typeof value !== "string") return null;

  const tauriDate = value
    .trim()
    .match(/^(\d{4}-\d{2}-\d{2}) (\d{2}:\d{2}:\d{2}(?:\.\d+)?) ([+-]\d{2}:\d{2}):00$/);
  if (!tauriDate) return null;

  const normalized = new Date(`${tauriDate[1]}T${tauriDate[2]}${tauriDate[3]}`);
  return Number.isNaN(normalized.getTime()) ? null : normalized;
}

function formatDate(value) {
  const date = parseDateValue(value);
  if (!date) return "-";
  return new Intl.DateTimeFormat(localeForLanguage(), {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit"
  }).format(date);
}

function formatTime(value) {
  const date = parseDateValue(value);
  if (!date) return "-";
  return new Intl.DateTimeFormat(localeForLanguage(), {
    hour: "2-digit",
    minute: "2-digit"
  }).format(date);
}

function renderSettingsStatus() {
  elements.settingsStatus.textContent = t("settingsSavedAt", { time: formatTime(settingsSavedAt) });
}

function scanDiagnosticsForProvider(providerId) {
  return scanDiagnostics.find((diagnostics) => diagnostics.provider === providerId) || null;
}

function activeScanDiagnostics() {
  return scanDiagnosticsForProvider(currentSettings.activeProvider);
}

function scanDiagnosticsProblem(diagnostics) {
  if (!diagnostics) {
    return { kind: "unverified", failedFiles: 0 };
  }
  const failedFiles = Number(diagnostics.failedFiles) || 0;
  if (diagnostics.cacheWriteSkipped) {
    return { kind: "write-skipped", failedFiles };
  }
  if (diagnostics.cacheWriteSucceeded === false) {
    return { kind: "write-failed", failedFiles };
  }
  if (failedFiles > 0) {
    return { kind: "scan-failed", failedFiles };
  }
  return null;
}

function scanDiagnosticsProblemText(problem, isRebuildResult = false) {
  if (!problem) return "";
  if (problem.kind === "write-skipped") {
    return t("cacheWriteSkipped", { count: formatNumber(problem.failedFiles) });
  }
  if (problem.kind === "write-failed") {
    return t("cacheWriteFailed");
  }
  if (problem.kind === "scan-failed") {
    return t("scanCompletedWithFailures", { count: formatNumber(problem.failedFiles) });
  }
  return isRebuildResult ? t("cacheRebuildUnverified") : "";
}

function renderScanDiagnostics() {
  const provider = PROVIDERS[currentSettings.activeProvider] || PROVIDERS.codex;
  const diagnostics = activeScanDiagnostics();
  const rebuildResult =
    cacheRebuildResult?.provider === currentSettings.activeProvider ? cacheRebuildResult : null;

  elements.scanProviderValue.textContent = provider.label;
  if (diagnostics) {
    elements.scanDiagnosticsSummary.textContent = t("scanSummary", {
      elapsed: formatNumber(diagnostics.elapsedMs),
      hits: formatNumber(diagnostics.cacheHits),
      files: formatNumber(diagnostics.totalFiles),
      rate: Math.round(Number(diagnostics.cacheHitRate) || 0)
    });
    elements.scanDiagnosticsMeta.textContent = t("scanMeta", {
      parsed: formatNumber(diagnostics.parsedFiles),
      deleted: formatNumber(diagnostics.deletedFiles),
      failed: formatNumber(diagnostics.failedFiles)
    });
  } else {
    elements.scanDiagnosticsSummary.textContent = t("noScanDiagnostics");
    elements.scanDiagnosticsMeta.textContent = "-";
  }

  const diagnosticsProblem =
    rebuildResult?.problem || (diagnostics ? scanDiagnosticsProblem(diagnostics) : null);
  if (cacheRebuildInProgress) {
    elements.rebuildCacheStatus.textContent = t("rebuildingCache");
  } else if (rebuildResult?.error) {
    elements.rebuildCacheStatus.textContent = rebuildResult.error;
  } else if (diagnosticsProblem) {
    elements.rebuildCacheStatus.textContent = scanDiagnosticsProblemText(
      diagnosticsProblem,
      Boolean(rebuildResult)
    );
  } else if (rebuildResult) {
    elements.rebuildCacheStatus.textContent = t("cacheRebuildComplete");
  } else {
    elements.rebuildCacheStatus.textContent = diagnostics ? t("scanCacheReady") : "-";
  }

  elements.rebuildCacheButton.disabled = currentLoading || cacheRebuildInProgress;
  elements.rebuildCacheButton.textContent = cacheRebuildInProgress ? t("rebuildingCache") : t("rebuildCache");
}

async function refreshScanDiagnostics() {
  try {
    const diagnostics = await aiUsage.getScanDiagnostics();
    scanDiagnostics = Array.isArray(diagnostics) ? diagnostics : [];
    renderScanDiagnostics();
    return true;
  } catch {
    scanDiagnostics = [];
    renderScanDiagnostics();
    return false;
  }
}

function updateTodayTokensMeter(percent) {
  const normalized = Number.isFinite(percent) ? Math.min(100, Math.max(0, percent)) : 0;
  const visiblePercent = normalized > 0 ? Math.max(3, normalized) : 0;
  const rounded = Math.round(normalized);
  const label = t("todayTokenUsageShare", { percent: rounded });
  elements.todayTokensMeter.style.setProperty("--today-token-progress", `${visiblePercent}%`);
  elements.todayTokensMeter.setAttribute("aria-valuenow", String(rounded));
  elements.todayTokensMeter.setAttribute("aria-label", label);
  elements.todayTokensMeter.setAttribute("title", label);
}

function costMetricsAvailable(stats = lastStats) {
  if (typeof stats?.featured?.costAvailable === "boolean") {
    return stats.featured.costAvailable;
  }
  return currentSettings.activeProvider === "codex";
}

function renderCostMetricLabels(chartDays) {
  elements.todayCostLabel.textContent = t("todayCost");
  elements.periodCostLabel.textContent = t("periodCost", { days: chartDays });
  elements.todayCostLabel.removeAttribute("title");
  elements.periodCostLabel.removeAttribute("title");
  elements.todayCost.removeAttribute("title");
  elements.periodCost.removeAttribute("title");
}

function renderCostMetricValues(stats) {
  if (costMetricsAvailable(stats)) {
    elements.todayCost.textContent = formatCurrency(stats.featured.todayCost);
    elements.periodCost.textContent = formatCurrency(stats.featured.periodCost);
    elements.todayCost.removeAttribute("title");
    elements.periodCost.removeAttribute("title");
  } else {
    elements.todayCost.textContent = t("notAvailable");
    elements.periodCost.textContent = t("notAvailable");
    elements.todayCostLabel.setAttribute("title", t("costUnavailable"));
    elements.periodCostLabel.setAttribute("title", t("costUnavailable"));
    elements.todayCost.setAttribute("title", t("costUnavailable"));
    elements.periodCost.setAttribute("title", t("costUnavailable"));
  }
}

function applyLanguage() {
  const lang = currentLanguage();
  const chartDays = currentSettings.chartDays || 30;

  document.documentElement.lang = lang === "zh" ? "zh-CN" : "en";
  elements.documentTitle.textContent = "Dial";
  elements.brandSubtitle.textContent = t("brandSubtitle");
  elements.primaryNav.setAttribute("aria-label", t("primaryNavigation"));
  elements.homeButton.textContent = t("summary");
  elements.trendButton.textContent = t("trend");
  elements.activityButton.textContent = t("activityTab");
  elements.sourceShortcutButton.textContent = t("switchDataSource");
  elements.settingsButton.setAttribute("aria-label", t("settings"));
  elements.settingsButton.setAttribute("title", t("settings"));
  elements.settingsBackLabel.textContent = t("backToApp");
  elements.settingsWindowTitle.textContent = t("settings");
  elements.settingsSearchInput.placeholder = t("settingsSearch");
  elements.settingsSearchInput.setAttribute("aria-label", t("settingsSearch"));
  elements.settingsNoResultsTitle.textContent = t("settingsNoResults");
  elements.settingsPersonalLabel.textContent = t("personal");
  elements.settingsProvidersLabel.textContent = t("providers");
  elements.settingsGeneralNavLabel.textContent = t("general");
  elements.settingsAppearanceNavLabel.textContent = t("appearance");
  elements.settingsChartNavLabel.textContent = t("chart");
  elements.settingsDataNavLabel.textContent = t("dataAndCache");
  elements.settingsUpdateNavLabel.textContent = t("updates");
  elements.settingsCodexNavLabel.textContent = PROVIDERS.codex.label;
  elements.settingsClaudeNavLabel.textContent = PROVIDERS.claude.label;
  elements.settingsCopilotNavLabel.textContent = PROVIDERS.copilot.label;
  elements.settingsCursorNavLabel.textContent = PROVIDERS.cursor.label;
  elements.settingsChatgptNavLabel.textContent = PROVIDERS.chatgpt.label;
  elements.repositoryLink.setAttribute("aria-label", t("repository"));
  elements.repositoryLink.setAttribute("title", t("repository"));
  elements.issueLink.setAttribute("aria-label", t("feedback"));
  elements.issueLink.setAttribute("title", t("feedback"));
  elements.sidebarProviderSection.setAttribute("aria-label", t("aiService"));
  elements.sidebarProviderLabel.textContent = t("aiService");
  elements.providerOptions.setAttribute("aria-label", t("aiService"));
  elements.sidebarUsageSection.setAttribute("aria-label", t("periodUsage"));
  elements.sidebarUsageLabel.textContent = t("remainingUsage");
  elements.updateButton.setAttribute("aria-label", t("updateAvailable", { version: updateInfo.version || "" }).trim());
  elements.updateButton.setAttribute("title", t("updateAvailable", { version: updateInfo.version || "" }).trim());
  elements.refreshButton.setAttribute("aria-label", t("refresh"));
  elements.refreshButton.setAttribute("title", t("refresh"));
  renderAutoRefreshCountdown();
  elements.overviewSourceLabel.textContent = t("dataSource");
  applyProviderEstimateText();
  renderCostMetricLabels(chartDays);
  elements.periodTokensLabel.textContent = t("periodTokens", { days: chartDays });
  elements.periodUsageMeta.dataset.periodLabel = t("periodAccumulated", { days: chartDays });
  elements.periodUsageMeta.textContent = t("localRecordsOnly");
  if (lastStats) {
    elements.periodTokensContext.textContent = t("periodTokenContext", {
      total: formatCompact(lastStats.totals.totalTokens),
      latest: formatCompact(lastStats.featured.latestTokenUsage)
    });
  }
  elements.todayTokensLabel.textContent = t("todayTokens");
  updateTodayTokensMeter(
    lastStats ? statsRatioPercent(lastStats.featured.todayTokens, lastStats.featured.periodTokens) : 0
  );
  elements.threadsTotalLabel.textContent = t("threadsTotal");
  elements.threadsTotalLabel.setAttribute("title", t("threadsTotalHint"));
  elements.threadsTotal.setAttribute("title", t("threadsTotalHint"));
  elements.threadsTotalMeta.textContent = t("allLocalRecords");
  elements.threadsTotalMeta.setAttribute("title", t("threadsTotalHint"));
  elements.threadsActiveLabel.textContent = t("threadsActive");
  elements.tokensTotalLabel.textContent = t("tokensTotal");
  elements.updatedThisWeekLabel.textContent = t("updatedThisWeek");
  elements.activityTitle.textContent = t("quotaConsumption");
  elements.rateLimitTitle.textContent = t("quotaWindows");
  for (const button of elements.chartModeButtons) {
    if (button.dataset.chartMode === "five-hour") {
      button.textContent = formatLimitLabel({ windowMinutes: 300 });
    } else if (button.dataset.chartMode === "weekly") {
      button.textContent = formatLimitLabel({ windowMinutes: 10080 });
    } else {
      button.textContent = "Token";
    }
  }
  renderChartHelp();
  elements.modelTitle.textContent = t("models");
  elements.sourceTitle.textContent = t("sources");
  elements.workspaceTitle.textContent = t("workspaces");
  elements.recentThreadsTitle.textContent = t("recentThreads");
  elements.usageInsightsTitle.textContent = t("usageInsights");
  elements.usageInsightsMeta.textContent = t("usageInsightsMeta");
  if (lastStats) {
    renderUsageInsights(lastStats);
  }
  elements.settingsPanelTitle.textContent = t("settingsGeneralTitle");
  elements.settingsGeneralTitle.textContent = t("settingsAppGroup");
  elements.settingsProvidersContentTitle.textContent = t("providers");
  elements.settingsCodexProviderTitle.textContent = PROVIDERS.codex.label;
  elements.settingsClaudeProviderTitle.textContent = PROVIDERS.claude.label;
  elements.settingsCopilotProviderTitle.textContent = PROVIDERS.copilot.label;
  elements.settingsCursorProviderTitle.textContent = PROVIDERS.cursor.label;
  elements.settingsChatgptProviderTitle.textContent = PROVIDERS.chatgpt.label;
  elements.settingsAppearanceTitle.textContent = t("appearance");
  elements.settingsChartTitle.textContent = t("settingsExperienceGroup");
  elements.settingsDataTitle.textContent = t("dataAndCache");
  elements.settingsUpdateTitle.textContent = t("updates");
  elements.scanProviderLabel.textContent = t("scanProvider");
  elements.latestScanLabel.textContent = t("latestScan");
  elements.cacheManagementLabel.textContent = t("scanCache");
  elements.rebuildCacheButton.textContent = cacheRebuildInProgress ? t("rebuildingCache") : t("rebuildCache");
  for (const label of elements.providerEnabledLabels) {
    label.textContent = t("providerEnabled");
  }
  for (const label of elements.providerToggleLabels) {
    label.textContent = t("providerEnabled");
  }
  elements.codexHomeLabel.textContent = t("dataFolder");
  elements.claudeHomeLabel.textContent = t("dataFolder");
  elements.copilotHomeLabel.textContent = t("dataFolder");
  elements.cursorHomeLabel.textContent = t("dataFolder");
  elements.chatgptHomeLabel.textContent = t("dataFolder");
  elements.chooseCodexHomeButton.textContent = t("chooseFolder");
  elements.chooseClaudeHomeButton.textContent = t("chooseFolder");
  elements.chooseCopilotHomeButton.textContent = t("chooseFolder");
  elements.chooseCursorHomeButton.textContent = t("chooseFolder");
  elements.chooseChatgptHomeButton.textContent = t("chooseFolder");
  elements.languageLabel.textContent = t("language");
  elements.languageAutoOption.textContent = t("followSystem");
  elements.languageZhOption.textContent = t("chinese");
  elements.languageEnOption.textContent = t("english");
  elements.autoRefreshLabel.textContent = t("autoRefresh");
  elements.autoRefreshEnabledLabel.textContent = t("autoRefreshEnabled");
  elements.autoRefreshSuffix.textContent = t("autoRefreshSuffix");
  elements.themeLabel.textContent = t("theme");
  elements.themeSystemOption.textContent = t("followSystem");
  elements.themeLightOption.textContent = t("light");
  elements.themeDarkOption.textContent = t("dark");
  elements.accentLabel.textContent = t("accentColor");
  elements.accentOptions.setAttribute("aria-label", t("accentColor"));
  elements.chartPeriodLabel.textContent = t("chartPeriod");
  elements.periodPresets.setAttribute("aria-label", t("chartPeriodPresets"));
  elements.period7Button.textContent = t("oneWeek");
  elements.period30Button.textContent = t("oneMonth");
  elements.period90Button.textContent = t("threeMonths");
  elements.daysSuffix.textContent = t("daysSuffix");
  elements.currentVersionLabel.textContent = t("currentVersion");
  elements.latestVersionLabel.textContent = t("latestVersion");
  elements.checkUpdateButton.textContent = updateCheckInProgress ? t("checkingUpdate") : t("checkForUpdates");
  elements.settingsInstallUpdateButton.textContent = updateInstalling ? t("installingUpdate") : t("installNow");
  elements.updateDialogEyebrow.textContent = t("updateDialogEyebrow");
  elements.updateDialogTitle.textContent = t("updateDialogTitle");
  elements.updateDialogSubtitle.textContent = t("updateDialogSubtitle");
  elements.closeUpdateDialogButton.setAttribute("aria-label", t("close"));
  elements.closeUpdateDialogButton.setAttribute("title", t("close"));
  elements.dialogCurrentVersionLabel.textContent = t("currentVersion");
  elements.dialogLatestVersionLabel.textContent = t("latestVersion");
  elements.dialogPublishedAtLabel.textContent = t("publishedAt");
  elements.updateNotesTitle.textContent = t("releaseNotes");
  elements.postponeUpdateButton.textContent = t("later");
  elements.installUpdateButton.textContent = updateInstalling ? t("installingUpdate") : t("installNow");
  renderProviderVisibility();
  renderScanDiagnostics();
  renderUpdateSurfaces();
  activateSettingsNav(activeSettingsSectionId);
  setView(currentView);
}

function applyTheme(theme) {
  const resolvedTheme =
    theme === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : theme;

  document.body.dataset.theme = resolvedTheme === "dark" ? "dark" : "light";
  syncProviderLogoTheme();
  if (lastStats) {
    renderUsageInsights(lastStats);
  }
}

function applyAccent(accentColor) {
  const resolvedAccent = accentColor || "blue";
  document.body.dataset.accent = resolvedAccent;
  for (const button of elements.accentButtons) {
    button.classList.toggle("active", button.dataset.accent === resolvedAccent);
  }
}

function updateStatusText() {
  if (updateInstalling) {
    return t("installingUpdate");
  }
  if (updateCheckInProgress) {
    return t("checkingUpdate");
  }
  if (updateCheckError) {
    return updateCheckError;
  }
  if (!updateInfo.supported) {
    return t("updateUnsupported");
  }
  if (updateInfo.available) {
    return t("updateAvailableStatus", { version: updateInfo.version || "" });
  }
  return t("noUpdateAvailable");
}

function renderUpdateDialog() {
  const version = updateInfo.version || "-";
  const currentVersion = updateInfo.currentVersion || "-";
  elements.dialogCurrentVersion.textContent = currentVersion;
  elements.dialogLatestVersion.textContent = version;
  elements.dialogPublishedAt.textContent = updateInfo.publishedAt ? formatDate(updateInfo.publishedAt) : "-";
  elements.updateNotes.textContent = updateInfo.notes?.trim() || t("noReleaseNotes");
  elements.installUpdateButton.disabled = updateInstalling;
  elements.installUpdateButton.textContent = updateInstalling ? t("installingUpdate") : t("installNow");
}

function openUpdateDialog() {
  if (!updateInfo.supported || !updateInfo.available) {
    return;
  }
  renderUpdateDialog();
  elements.updateDialog.hidden = false;
  elements.installUpdateButton.focus();
}

function closeUpdateDialog() {
  elements.updateDialog.hidden = true;
}

function renderUpdateButton() {
  const version = updateInfo.version || "";
  const isSettings = currentView === "settings";
  const shouldShow = Boolean(updateInfo.supported && updateInfo.available && !isSettings);

  elements.updateButton.hidden = !shouldShow;
  elements.updateButton.disabled = currentLoading || updateInstalling;
  elements.updateButton.textContent = updateInstalling ? t("installingUpdate") : t("updateAvailable", { version });
  elements.updateButton.setAttribute("aria-label", t("updateAvailable", { version }));
  elements.updateButton.setAttribute("title", t("updateAvailable", { version }));
}

function renderUpdateSurfaces() {
  renderUpdateButton();
  elements.currentVersionValue.textContent = updateInfo.currentVersion || "-";
  elements.latestVersionValue.textContent = updateInfo.version || "-";
  elements.settingsUpdateStatus.textContent = updateStatusText();
  elements.checkUpdateButton.disabled = currentLoading || updateCheckInProgress || updateInstalling;
  elements.checkUpdateButton.textContent = updateCheckInProgress ? t("checkingUpdate") : t("checkForUpdates");
  elements.settingsInstallUpdateButton.hidden = !Boolean(updateInfo.supported && updateInfo.available);
  elements.settingsInstallUpdateButton.disabled = currentLoading || updateInstalling;
  elements.settingsInstallUpdateButton.textContent = updateInstalling
    ? t("installingUpdate")
    : t("updateAvailable", { version: updateInfo.version || "" });
  if (!elements.updateDialog.hidden) {
    renderUpdateDialog();
  }
}

function setView(view) {
  currentView = view;
  document.body.dataset.view = view;
  const isSettings = view === "settings";
  const provider = PROVIDERS[currentSettings.activeProvider] || PROVIDERS.codex;
  elements.homeView.hidden = isSettings;
  elements.settingsView.hidden = !isSettings;
  elements.refreshButton.hidden = isSettings;
  elements.settingsButton.classList.toggle("active", isSettings);
  elements.viewEyebrow.textContent = isSettings ? t("preferences") : `${provider.label} ${t("usage")}`;
  elements.viewTitle.textContent = isSettings ? t("settings") : t("overview");
  if (isSettings) {
    setProviderMenuOpen(false);
  }
  updateHomeSectionNav();
  renderUpdateSurfaces();
  renderAutoRefreshCountdown();
  syncRateLimitCountdownTimer();
}

function settingsPanelGroupTitle(sectionId) {
  return [
    "settingsGeneralSection",
    "settingsAppearanceSection",
    "settingsChartSection",
    "settingsDataSection",
    "settingsUpdateSection"
  ].includes(sectionId)
    ? t("personal")
    : t("providers");
}

function updateHomeSectionNav() {
  const sectionButtons = {
    summary: elements.homeButton,
    trend: elements.trendButton,
    activity: elements.activityButton
  };
  for (const [section, button] of Object.entries(sectionButtons)) {
    button.classList.toggle("active", currentView === "home" && currentHomeSection === section);
  }
}

function setHomeSection(section, options = {}) {
  currentHomeSection = section;
  setView("home");
  const target =
    section === "trend"
      ? document.getElementById("usageChartPanel")
      : section === "activity"
        ? document.getElementById("activitySection")
        : elements.homeView;
  if (options.scroll !== false) {
    if (section === "summary") {
      window.scrollTo({ top: 0, behavior: "smooth" });
    } else {
      target?.scrollIntoView({ behavior: "smooth", block: "center" });
    }
  }
}

function setProviderMenuOpen(open) {
  const shouldOpen = Boolean(open && currentView === "home");
  elements.sidebarProviderSection.hidden = !shouldOpen;
  elements.providerMenuButton.setAttribute("aria-expanded", String(shouldOpen));
}

const PERSONAL_SETTINGS_SECTION_IDS = [
  "settingsGeneralSection",
  "settingsAppearanceSection",
  "settingsChartSection",
  "settingsDataSection",
  "settingsUpdateSection"
];

function isProviderSettingsSection(sectionId) {
  return !PERSONAL_SETTINGS_SECTION_IDS.includes(sectionId);
}

function providerForSettingsSection(sectionId) {
  const button = elements.settingsProviderNavButtons.find(
    (candidate) => candidate.dataset.settingsSection === sectionId
  );
  return button?.dataset.settingsProvider || null;
}

function settingsPanelTitle(sectionId) {
  if (sectionId === "settingsGeneralSection") return t("settingsGeneralTitle");
  if (sectionId === "settingsAppearanceSection") return t("appearance");
  if (sectionId === "settingsChartSection") return t("chart");
  if (sectionId === "settingsDataSection") return t("dataAndCache");
  if (sectionId === "settingsUpdateSection") return t("updates");
  const providerId = providerForSettingsSection(sectionId);
  return providerId ? PROVIDERS[providerId]?.label || t("providers") : t("providers");
}

function settingsPanelDescription(sectionId) {
  if (sectionId === "settingsGeneralSection") return t("settingsGeneralDescription");
  if (sectionId === "settingsAppearanceSection") return t("settingsAppearanceDescription");
  if (sectionId === "settingsChartSection") return t("settingsChartDescription");
  if (sectionId === "settingsDataSection") return t("settingsDataDescription");
  if (sectionId === "settingsUpdateSection") return t("settingsUpdateDescription");
  return t("settingsProvidersDescription");
}

function updateSettingsSectionHeadings() {
  elements.settingsGeneralTitle.textContent = t("settingsAppGroup");
  elements.settingsAppearanceTitle.textContent = t("appearance");
  elements.settingsChartTitle.textContent =
    activeSettingsSectionId === "settingsGeneralSection" ? t("settingsExperienceGroup") : t("chart");
  elements.settingsDataTitle.textContent = t("dataAndCache");
  elements.settingsUpdateTitle.textContent = t("updates");
}

function updateSettingsSectionVisibility() {
  const query = elements.settingsSearchInput.value.trim();
  const normalizedQuery = query.toLocaleLowerCase();
  const isSearching = Boolean(normalizedQuery);
  const isProviderSection = isProviderSettingsSection(activeSettingsSectionId);
  let visibleCount = 0;

  for (const section of elements.settingsSections) {
    let visible;
    if (isSearching) {
      visible = section.textContent.toLocaleLowerCase().includes(normalizedQuery);
    } else if (activeSettingsSectionId === "settingsGeneralSection") {
      visible = ["settingsGeneralSection", "settingsChartSection"].includes(section.id);
    } else {
      visible = section.id === activeSettingsSectionId;
    }
    section.hidden = !visible;
    if (visible) visibleCount += 1;
  }

  elements.settingsProviderTabs.hidden = isSearching || !isProviderSection;
  elements.settingsNoResults.hidden = !isSearching || visibleCount > 0;
  if (isSearching) {
    elements.settingsPanelTitle.textContent = t("settingsSearchResults");
    elements.settingsSectionDescription.textContent = t("settingsSearchDescription", { query });
  } else {
    elements.settingsPanelTitle.textContent = settingsPanelTitle(activeSettingsSectionId);
    elements.settingsSectionDescription.textContent = settingsPanelDescription(activeSettingsSectionId);
  }
  updateSettingsSectionHeadings();
}

function clearSettingsSearch() {
  if (!elements.settingsSearchInput.value) return;
  elements.settingsSearchInput.value = "";
}

function activateSettingsNav(sectionId, options = {}) {
  activeSettingsSectionId = sectionId;
  const isProviderSection = isProviderSettingsSection(sectionId);
  for (const button of elements.settingsNavButtons) {
    const isActive = button.dataset.settingsSection === sectionId;
    button.classList.toggle("active", isActive);
    button.setAttribute("aria-current", isActive ? "page" : "false");
  }
  elements.settingsProvidersTab.classList.toggle("active", isProviderSection);
  elements.settingsProvidersTab.setAttribute("aria-current", isProviderSection ? "page" : "false");
  updateSettingsSectionVisibility();
  if (options.scroll) {
    elements.settingsContent.scrollTo({ top: 0, behavior: "smooth" });
  }
}

function renderProviderVisibility() {
  for (const button of elements.providerButtons) {
    const isEnabled = isProviderEnabled(button.dataset.provider);
    button.hidden = !isEnabled;
    button.setAttribute("aria-hidden", String(!isEnabled));
    const isActive = button.dataset.provider === currentSettings.activeProvider;
    button.classList.toggle("active", isActive);
    button.setAttribute("aria-checked", String(isActive));
  }

  for (const button of elements.settingsProviderNavButtons) {
    const isEnabled = isProviderEnabled(button.dataset.settingsProvider);
    button.classList.toggle("inactive-provider", !isEnabled);
  }
}

function setLoading(isLoading) {
  currentLoading = isLoading;
  elements.refreshButton.disabled = isLoading;
  elements.updateButton.disabled = isLoading || updateInstalling;
  elements.checkUpdateButton.disabled = isLoading || updateCheckInProgress || updateInstalling;
  elements.settingsInstallUpdateButton.disabled = isLoading || updateInstalling;
  elements.installUpdateButton.disabled = updateInstalling;
  elements.chooseCodexHomeButton.disabled = isLoading || !isProviderEnabled("codex");
  elements.chooseClaudeHomeButton.disabled = isLoading || !isProviderEnabled("claude");
  elements.chooseCopilotHomeButton.disabled = isLoading || !isProviderEnabled("copilot");
  elements.chooseCursorHomeButton.disabled = isLoading || !isProviderEnabled("cursor");
  elements.chooseChatgptHomeButton.disabled = isLoading || !isProviderEnabled("chatgpt");
  elements.rebuildCacheButton.disabled = isLoading || cacheRebuildInProgress;
  elements.refreshButton.classList.toggle("loading", isLoading);
}

function renderError(message) {
  elements.errorPanel.hidden = !message;
  elements.errorPanel.textContent = message || "";
}

function providerHome(providerId, settings = currentSettings) {
  if (providerId === "claude") return settings.claudeHome || "";
  if (providerId === "copilot") return settings.copilotHome || "";
  if (providerId === "cursor") return settings.cursorHome || "";
  if (providerId === "chatgpt") return settings.chatgptHome || "";
  return settings.codexHome || "";
}

function statsCacheKey(providerId = currentSettings.activeProvider, settings = currentSettings) {
  return [providerId, settings.chartDays || 30, providerHome(providerId, settings)].join("\u001f");
}

function cacheStats(providerId, stats, settings = currentSettings) {
  const key = statsCacheKey(providerId, settings);
  statsCache.delete(key);
  statsCache.set(key, stats);
  while (statsCache.size > MAX_STATS_CACHE_ENTRIES) {
    statsCache.delete(statsCache.keys().next().value);
  }
}

function cachedStats(providerId = currentSettings.activeProvider, settings = currentSettings) {
  return statsCache.get(statsCacheKey(providerId, settings)) || null;
}

function clearSkeletons() {
  for (const node of document.querySelectorAll(".skeleton-line, .skeleton-block")) {
    node.classList.remove("skeleton-line", "skeleton-block");
    node.style.removeProperty("--skeleton-width");
  }
}

function markSkeleton(element, width = "72%") {
  element.textContent = "";
  element.classList.add("skeleton-line");
  element.style.setProperty("--skeleton-width", width);
}

function appendSkeletonRows(container, count, kind = "rank") {
  container.replaceChildren();
  for (let index = 0; index < count; index += 1) {
    const row = document.createElement("div");
    row.className = kind === "thread" ? "thread-row" : "rank-row";

    if (kind === "thread") {
      const main = document.createElement("div");
      const title = document.createElement("strong");
      const meta = document.createElement("span");
      markSkeleton(title, `${68 - index * 4}%`);
      markSkeleton(meta, `${52 - index * 3}%`);
      main.append(title, meta);
      const value = document.createElement("span");
      markSkeleton(value, "42px");
      row.append(main, value);
    } else {
      const label = document.createElement("span");
      label.className = "rank-label";
      const value = document.createElement("span");
      value.className = "rank-value";
      markSkeleton(label, `${64 - index * 5}%`);
      markSkeleton(value, "38px");
      row.append(label, value);
    }

    container.append(row);
  }
}

function setChartScale(daysCount) {
  elements.dailyChart.style.setProperty("--days", daysCount);
  const gap = daysCount > 180 ? 0 : daysCount > 60 ? 2 : 6;
  elements.dailyChart.style.setProperty("--chart-gap", `${gap}px`);
}

function renderChartSkeleton(daysCount) {
  const count = Math.min(12, Math.max(7, daysCount));
  renderUsageBarChart({
    items: Array.from({ length: count }, (_, index) => ({
      label: "",
      value: 18 + ((index * 17) % 54),
      displayValue: "",
      secondaryValue: "",
      showLabel: false
    })),
    maxValue: 100,
    formatAxis: () => "",
    referenceValue: null,
    referenceLabel: ""
  });
  for (const bar of elements.dailyChart.querySelectorAll(".chart-bar")) {
    bar.disabled = true;
    bar.classList.add("loading-bar");
  }
}

function renderStatsSkeleton(providerId = currentSettings.activeProvider) {
  clearSkeletons();
  renderError(null);

  const provider = PROVIDERS[providerId] || PROVIDERS.codex;
  const chartDays = currentSettings.chartDays || 30;
  elements.overviewProvider.textContent = provider.label;
  setProviderImage(elements.overviewProviderLogo, providerId);
  setProviderImage(elements.headerProviderLogo, providerId);
  elements.headerProviderLabel.textContent = provider.label;
  elements.overviewAccountName.textContent = provider.label;
  elements.overviewAccountPlan.textContent = provider.label;
  applyProviderEstimateText(providerId);
  renderCostMetricLabels(chartDays);
  elements.periodTokensLabel.textContent = t("periodTokens", { days: chartDays });
  elements.periodUsageMeta.dataset.periodLabel = t("periodAccumulated", { days: chartDays });
  elements.activityTitle.textContent = t("activityTrend", { days: chartDays });
  elements.overviewPeriodLabel.textContent = t("daysPeriod", { days: chartDays });
  updateTodayTokensMeter(0);
  elements.accountInitials.textContent = provider.initials;
  elements.accountName.textContent = provider.label;
  markSkeleton(elements.accountPlan, "64px");
  markSkeleton(elements.todayCost, "86px");
  markSkeleton(elements.periodCost, "86px");
  markSkeleton(elements.periodUsageMeta, "110px");
  markSkeleton(elements.periodTokens, "78px");
  markSkeleton(elements.periodTokensContext, "172px");
  markSkeleton(elements.todayTokens, "72px");
  markSkeleton(elements.threadsTotal, "48px");
  markSkeleton(elements.threadsActive, "48px");
  markSkeleton(elements.tokensTotal, "64px");
  markSkeleton(elements.updatedThisWeek, "48px");
  markSkeleton(elements.sidebarRemainingUsage, "46px");
  markSkeleton(elements.sidebarPeriodMeta, "116px");
  markSkeleton(elements.lastUpdated, "118px");
  markSkeleton(elements.rateLimitUpdated, "94px");

  elements.chartLegend.replaceChildren();
  elements.chartInterpretation.textContent = t("chartNoHistory");
  renderChartSkeleton(10);
  renderRateLimits(null);
  renderUsageInsights(null, chartDays);
  appendSkeletonRows(elements.modelList, 3);
  appendSkeletonRows(elements.sourceList, 3);
  appendSkeletonRows(elements.workspaceList, 4);
  appendSkeletonRows(elements.recentThreads, 4, "thread");
}

function renderSettings() {
  currentSettings.language = currentSettings.language || "auto";
  currentSettings.enabledProviders = enabledProviders();
  if (!isProviderEnabled(currentSettings.activeProvider)) {
    currentSettings.activeProvider = firstEnabledProvider();
  }
  elements.codexHomeValue.textContent = currentSettings.codexHome || PROVIDERS.codex.defaultHome;
  elements.claudeHomeValue.textContent = currentSettings.claudeHome || PROVIDERS.claude.defaultHome;
  elements.copilotHomeValue.textContent = currentSettings.copilotHome || PROVIDERS.copilot.defaultHome;
  elements.cursorHomeValue.textContent = currentSettings.cursorHome || PROVIDERS.cursor.defaultHome;
  elements.chatgptHomeValue.textContent = currentSettings.chatgptHome || PROVIDERS.chatgpt.defaultHome;
  for (const input of elements.enabledProviderInputs) {
    input.checked = isProviderEnabled(input.dataset.enabledProvider);
  }
  renderProviderVisibility();
  elements.languageSelect.value = currentSettings.language;
  currentSettings.autoRefreshEnabled = Boolean(currentSettings.autoRefreshEnabled);
  currentSettings.autoRefreshMinutes = normalizeAutoRefreshMinutes(currentSettings.autoRefreshMinutes);
  elements.autoRefreshEnabledInput.checked = currentSettings.autoRefreshEnabled;
  elements.autoRefreshInput.disabled = !currentSettings.autoRefreshEnabled;
  elements.autoRefreshInput.min = "1";
  elements.autoRefreshInput.max = String(MAX_AUTO_REFRESH_MINUTES);
  elements.autoRefreshInput.value = currentSettings.autoRefreshMinutes;
  elements.themeSelect.value = currentSettings.theme;
  applyAccent(currentSettings.accentColor);
  elements.chartDaysInput.min = String(MIN_CHART_DAYS);
  elements.chartDaysInput.max = String(MAX_CHART_DAYS);
  elements.chartDaysInput.value = currentSettings.chartDays;
  elements.overviewPeriodLabel.textContent = t("daysPeriod", { days: currentSettings.chartDays });
  elements.sidebarPeriodMeta.textContent = t("daysPeriod", { days: currentSettings.chartDays });
  for (const button of elements.periodButtons) {
    button.classList.toggle("active", Number(button.dataset.days) === Number(currentSettings.chartDays));
  }
  renderSettingsStatus();
  renderScanDiagnostics();
  applyTheme(currentSettings.theme);
  applyLanguage();
  setLoading(currentLoading);
}

function renderRankList(container, items, options = {}) {
  container.replaceChildren();

  if (!items.length) {
    const empty = document.createElement("p");
    empty.className = "empty";
    empty.textContent = t("emptyData");
    container.append(empty);
    return;
  }

  const max = Math.max(...items.map((item) => item.value), 1);

  for (const item of items) {
    const row = document.createElement("div");
    row.className = "rank-row";

    const label = document.createElement("span");
    label.className = "rank-label";
    label.textContent = options.formatName ? options.formatName(item.name) : item.name;

    const value = document.createElement("span");
    value.className = "rank-value";
    value.textContent = options.compact ? formatCompact(item.value) : formatNumber(item.value);

    const bar = document.createElement("span");
    bar.className = "rank-bar";
    bar.style.setProperty("--value", `${Math.max(4, (item.value / max) * 100)}%`);

    row.append(label, value, bar);
    container.append(row);
  }
}

function renderRateLimits(rateLimits) {
  elements.rateLimitList.replaceChildren();
  const windows = availableRateLimitWindows(rateLimits).sort((a, b) => a.windowMinutes - b.windowMinutes);

  elements.rateLimitUpdated.textContent = rateLimits?.updatedAt
    ? t("updatedAt", { date: formatDate(rateLimits.updatedAt) })
    : t("waitingLimitData");

  for (const limit of windows) {
    const minutes = Number(limit.windowMinutes);
    const pace = rateLimitPace(limit);
    const row = document.createElement("div");
    row.className = "limit-row";

    const icon = document.createElement("span");
    icon.className = `ui-icon limit-icon ${minutes === 300 ? "icon-clock" : "icon-calendar"}`;
    icon.setAttribute("aria-hidden", "true");

    const label = document.createElement("strong");
    label.textContent = formatLimitLabel({ windowMinutes: minutes });

    const value = document.createElement("span");
    value.className = "limit-value";
    const valueNumber = document.createElement("b");
    valueNumber.textContent = formatPercent(pace.remainingPercent);
    value.append(valueNumber, document.createTextNode(currentLanguage() === "zh" ? " 剩余" : " remaining"));

    const meter = document.createElement("div");
    meter.className = "limit-meter";
    meter.setAttribute("role", "progressbar");
    meter.setAttribute("aria-valuemin", "0");
    meter.setAttribute("aria-valuemax", "100");
    meter.setAttribute("aria-valuenow", String(Math.round(pace.remainingPercent)));
    meter.setAttribute("aria-label", `${label.textContent} ${value.textContent}`);
    appendSegmentedMeter(meter, pace.remainingPercent);

    const meta = document.createElement("div");
    meta.className = "limit-meta";
    const countdown = document.createElement("strong");
    const projection = document.createElement("span");
    countdown.dataset.resetCountdown = limit.resetsAt || "";
    countdown.textContent = formatUpdateCountdown(limit.resetsAt);
    projection.textContent =
      pace.projectedRemainingPercent === null
        ? `${formatResetTime(limit)} ${currentLanguage() === "zh" ? "重置" : "reset"}`
        : t("estimatedRemainingAtReset", { percent: Math.round(pace.projectedRemainingPercent) });
    meta.append(countdown, projection);
    row.append(icon, label, value, meter, meta);
    elements.rateLimitList.append(row);
  }

  updateRateLimitCountdowns();
  syncRateLimitCountdownTimer();
}

function appendSegmentedMeter(meter, percent) {
  const remaining = clampPercent(percent);
  const segmentCount = 10;
  for (let index = 0; index < segmentCount; index += 1) {
    const segment = document.createElement("span");
    segment.className = "limit-segment";
    const fill = document.createElement("i");
    fill.style.width = `${Math.max(0, Math.min(10, remaining - index * 10)) * 10}%`;
    segment.append(fill);
    meter.append(segment);
  }
}

function renderActivity(activity) {
  elements.rateLimitList.replaceChildren();
  const windows = availableActivityWindows(activity);

  if (!windows.length) {
    const empty = document.createElement("p");
    empty.className = "empty";
    empty.textContent = t("emptyData");
    elements.rateLimitList.append(empty);
    elements.rateLimitUpdated.textContent = "-";
    syncRateLimitCountdownTimer();
    return;
  }

  elements.rateLimitUpdated.textContent = activity.updatedAt
    ? t("updatedAt", { date: formatDate(activity.updatedAt) })
    : "-";

  for (const window of windows) {
    const row = document.createElement("div");
    row.className = "limit-row activity-row";

    const icon = document.createElement("span");
    icon.className = `ui-icon limit-icon ${Number(window.windowMinutes) < 10080 ? "icon-clock" : "icon-calendar"}`;
    icon.setAttribute("aria-hidden", "true");

    const label = document.createElement("strong");
    label.textContent = formatLimitLabel(window);

    const count = document.createElement("span");
    count.className = "limit-value";
    const countNumber = document.createElement("b");
    countNumber.textContent = formatNumber(window.count);
    count.append(countNumber, document.createTextNode(currentLanguage() === "zh" ? " 次活动" : " activities"));

    const meter = document.createElement("div");
    meter.className = "limit-meter";
    appendSegmentedMeter(meter, 0);

    const meta = document.createElement("div");
    meta.className = "limit-meta";
    const rolling = document.createElement("strong");
    rolling.textContent = t("rollingWindow");
    const source = document.createElement("span");
    source.textContent = t("localRecordsOnly");
    meta.append(rolling, source);

    row.append(icon, label, count, meter, meta);
    elements.rateLimitList.append(row);
  }

  syncRateLimitCountdownTimer();
}

function rateLimitWindowByMinutes(stats, windowMinutes) {
  return availableRateLimitWindows(stats?.rateLimits).find(
    (window) => Number(window.windowMinutes) === Number(windowMinutes)
  );
}

function rateLimitHistoryByMinutes(stats, windowMinutes) {
  if (!Array.isArray(stats?.rateLimitHistory)) return null;
  return stats.rateLimitHistory.find((series) => Number(series.windowMinutes) === Number(windowMinutes)) || null;
}

function chartModeWindowMinutes(mode) {
  if (mode === "five-hour") return 300;
  if (mode === "weekly") return 10080;
  return null;
}

function syncChartMode(stats) {
  for (const button of elements.chartModeButtons) {
    const minutes = chartModeWindowMinutes(button.dataset.chartMode);
    const unavailable = minutes !== null && !rateLimitWindowByMinutes(stats, minutes);
    button.hidden = unavailable;
    button.disabled = unavailable;
  }

  const currentButton = elements.chartModeButtons.find((button) => button.dataset.chartMode === currentChartMode);
  if (!currentButton || currentButton.disabled || currentButton.hidden) {
    currentChartMode = "tokens";
  }

  for (const button of elements.chartModeButtons) {
    const isSelected = button.dataset.chartMode === currentChartMode;
    button.setAttribute("aria-selected", String(isSelected));
    button.tabIndex = isSelected ? 0 : -1;
  }
}

function buildTokenBars(stats, daysCount = null) {
  const source = Array.isArray(stats?.dailySeries) ? stats.dailySeries : [];
  const days = daysCount ? source.slice(-daysCount) : source;
  return {
    items: days.map((day, index) => ({
      label: day.label || day.date,
      value: Number(day.tokens) || 0,
      displayValue: `${formatCompact(day.tokens)} ${t("tokens")}`,
      secondaryValue: formatCurrency(day.cost),
      showLabel: days.length <= 14 || index % Math.max(1, Math.ceil(days.length / 7)) === 0
    })),
    maxValue: Math.max(...days.map((day) => Number(day.tokens) || 0), 1),
    formatAxis: (value) => formatCompact(value),
    referenceValue: null,
    interpretation: t("localTokenSummary", { value: formatCompact(days.reduce((sum, day) => sum + (Number(day.tokens) || 0), 0)) }),
    sourceLabel: t("localTokenActivity"),
    legend: [{ label: t("actualConsumption"), estimated: false }]
  };
}

function buildRateLimitHistoryPending(stats, windowMinutes) {
  const bucketCount = windowMinutes === 300 ? 10 : 7;
  const limit = rateLimitWindowByMinutes(stats, windowMinutes);
  const resetMs = new Date(limit?.resetsAt || "").getTime();
  const endMs = Number.isFinite(resetMs) ? resetMs : Date.now();
  const startMs = endMs - windowMinutes * 60 * 1000;
  const bucketMs = (endMs - startMs) / bucketCount;

  return {
    items: Array.from({ length: bucketCount }, (_, index) => {
      const timestamp = new Date(startMs + index * bucketMs);
      const label = windowMinutes === 300
        ? new Intl.DateTimeFormat(localeForLanguage(), { hour: "2-digit", minute: "2-digit" }).format(timestamp)
        : new Intl.DateTimeFormat(localeForLanguage(), { weekday: "short" }).format(timestamp);
      return {
        label,
        value: 0,
        displayValue: "0%",
        secondaryValue: t("chartNoHistory"),
        showLabel: windowMinutes !== 300 || index % 2 === 0
      };
    }),
    maxValue: 100,
    formatAxis: (value) => `${Math.round(value)}%`,
    referenceValue: null,
    interpretation: t("chartNoHistory"),
    sourceLabel: t("quotaSnapshot"),
    legend: [{ label: t("actualConsumption"), estimated: false }]
  };
}

function buildRateLimitBars(stats, windowMinutes) {
  const limit = rateLimitWindowByMinutes(stats, windowMinutes);
  const history = rateLimitHistoryByMinutes(stats, windowMinutes);
  const bucketCount = windowMinutes === 300 ? 10 : 7;
  const resetMs = new Date(limit?.resetsAt || "").getTime();
  const endMs = Number.isFinite(resetMs) ? resetMs : Date.now();
  const startMs = endMs - windowMinutes * 60 * 1000;
  const nowMs = Math.min(Date.now(), endMs);
  const bucketMs = (endMs - startMs) / bucketCount;
  const values = Array.from({ length: bucketCount }, () => 0);
  const estimated = Array.from({ length: bucketCount }, () => false);
  const points = (history?.points || [])
    .map((point) => ({ ...point, timestampMs: new Date(point.timestamp).getTime() }))
    .filter((point) => Number.isFinite(point.timestampMs) && point.timestampMs >= startMs && point.timestampMs <= nowMs)
    .sort((a, b) => a.timestampMs - b.timestampMs);

  let previous = null;
  for (const point of points) {
    let delta = 0;
    if (previous) {
      delta = Number(point.usedPercent) >= Number(previous.usedPercent)
        ? Number(point.usedPercent) - Number(previous.usedPercent)
        : Number(point.usedPercent);
    } else if (point.timestampMs - startMs <= bucketMs * 1.25) {
      delta = Number(point.usedPercent) || 0;
    }
    previous = point;
    if (delta <= 0) continue;
    const index = Math.min(bucketCount - 1, Math.max(0, Math.floor((point.timestampMs - startMs) / bucketMs)));
    values[index] += delta;
  }

  const elapsedFraction = Math.max(0.01, Math.min(1, (nowMs - startMs) / (endMs - startMs)));
  const officialUsed = clampPercent(limit?.usedPercent);
  const projectedTotal = Math.min(100, officialUsed / elapsedFraction);
  const projectedAdditional = Math.max(0, projectedTotal - officialUsed);
  const firstFutureBucket = Math.min(bucketCount, Math.max(0, Math.ceil((nowMs - startMs) / bucketMs)));
  const futureCount = Math.max(0, bucketCount - firstFutureBucket);
  if (futureCount > 0 && projectedAdditional > 0) {
    const perBucket = projectedAdditional / futureCount;
    for (let index = firstFutureBucket; index < bucketCount; index += 1) {
      values[index] = perBucket;
      estimated[index] = true;
    }
  }

  const visibleActual = values.reduce((sum, value, index) => sum + (estimated[index] ? 0 : value), 0);
  const referenceValue = 100 / bucketCount;
  const maxValue = Math.max(...values, referenceValue, 1) * 1.12;
  const items = values.map((value, index) => {
    const timestamp = new Date(startMs + index * bucketMs);
    const label = windowMinutes === 300
      ? new Intl.DateTimeFormat(localeForLanguage(), { hour: "2-digit", minute: "2-digit" }).format(timestamp)
      : new Intl.DateTimeFormat(localeForLanguage(), { weekday: "short" }).format(timestamp);
    return {
      label,
      value,
      estimated: estimated[index],
      displayValue: `${value.toFixed(value >= 10 ? 0 : 1).replace(/\.0$/, "")}%`,
      secondaryValue: estimated[index] ? t("estimatedConsumption") : t("actualConsumption"),
      showLabel: windowMinutes !== 300 || index % 2 === 0
    };
  });

  return {
    items,
    maxValue,
    formatAxis: (value) => `${Math.round(value)}%`,
    referenceValue,
    referenceLabel: currentLanguage() === "zh"
      ? `建议速度 ${referenceValue.toFixed(0)}% / ${windowMinutes === 300 ? "30 分钟" : "天"}`
      : `Suggested pace ${referenceValue.toFixed(0)}% / ${windowMinutes === 300 ? "30 min" : "day"}`,
    interpretation: points.length > 1
      ? t("visibleSnapshotConsumption", { percent: Math.round(visibleActual), official: Math.round(officialUsed) })
      : t("chartNoHistory"),
    sourceLabel: t("quotaSnapshot"),
    legend: [
      { label: t("actualConsumption"), estimated: false },
      { label: t("estimatedConsumption"), estimated: true }
    ]
  };
}

function renderChartLegend(items) {
  elements.chartLegend.replaceChildren();
  for (const item of items) {
    const label = document.createElement("span");
    const swatch = document.createElement("i");
    swatch.className = `legend-swatch${item.estimated ? " estimated" : ""}`;
    swatch.setAttribute("aria-hidden", "true");
    label.append(swatch, document.createTextNode(item.label));
    elements.chartLegend.append(label);
  }
}

function renderUsageBarChart(config) {
  elements.dailyChart.replaceChildren();
  const yAxis = document.createElement("div");
  yAxis.className = "chart-y-axis";
  const plot = document.createElement("div");
  plot.className = "chart-plot";
  plot.style.setProperty("--days", String(Math.max(config.items.length, 1)));
  plot.style.setProperty("--chart-gap", config.items.length > 30 ? "2px" : "7px");

  const gridlines = document.createElement("div");
  gridlines.className = "chart-gridlines";
  for (const fraction of [0, 0.25, 0.5, 0.75, 1]) {
    const axisLabel = document.createElement("span");
    axisLabel.style.setProperty("--axis-position", `${fraction * 100}%`);
    axisLabel.textContent = config.formatAxis(config.maxValue * fraction);
    yAxis.append(axisLabel);

    const line = document.createElement("span");
    line.className = "chart-gridline";
    line.style.bottom = `${fraction * 100}%`;
    gridlines.append(line);
  }
  plot.append(gridlines);

  if (Number.isFinite(config.referenceValue) && config.referenceValue > 0) {
    const reference = document.createElement("div");
    reference.className = "chart-reference-line";
    reference.style.bottom = `${Math.min(100, (config.referenceValue / config.maxValue) * 100)}%`;
    const referenceLabel = document.createElement("span");
    referenceLabel.textContent = config.referenceLabel;
    reference.append(referenceLabel);
    plot.append(reference);
  }

  for (const item of config.items) {
    const column = document.createElement("div");
    column.className = "chart-column";
    const bar = document.createElement("button");
    const numericValue = Math.max(0, Number(item.value) || 0);
    const numericMax = Math.max(1, Number(config.maxValue) || 1);
    const valueRatio = Math.min(1, numericValue / numericMax);
    const fillIntensity = Math.round(8 + valueRatio * 92);
    const borderIntensity = Math.round(32 + valueRatio * 68);
    const estimatedIntensity = Math.round(6 + valueRatio * 34);
    bar.type = "button";
    bar.className = `chart-bar${item.estimated ? " estimated" : ""}`;
    bar.style.height = item.value > 0 ? `${Math.max(2, (item.value / config.maxValue) * 100)}%` : "2px";
    bar.style.setProperty("--bar-intensity", `${fillIntensity}%`);
    bar.style.setProperty("--bar-border-intensity", `${borderIntensity}%`);
    bar.style.setProperty("--bar-estimated-intensity", `${estimatedIntensity}%`);
    bar.setAttribute("aria-label", `${item.label}: ${item.displayValue}, ${item.secondaryValue}`);
    bar.addEventListener("mouseenter", (event) => showUsageChartTooltip(event, item));
    bar.addEventListener("mousemove", (event) => moveChartTooltip(event));
    bar.addEventListener("mouseleave", hideChartTooltip);
    bar.addEventListener("focus", (event) => showUsageChartTooltip(event, item));
    bar.addEventListener("blur", hideChartTooltip);
    column.append(bar);
    if (item.showLabel) {
      const label = document.createElement("span");
      label.className = "chart-x-label";
      label.textContent = item.label;
      column.append(label);
    }
    plot.append(column);
  }

  elements.dailyChart.append(yAxis, plot);
}

function renderUsageChart(stats) {
  syncChartMode(stats);
  let config;
  if (currentChartMode === "tokens") {
    config = buildTokenBars(stats);
  } else {
    const minutes = chartModeWindowMinutes(currentChartMode);
    const history = rateLimitHistoryByMinutes(stats, minutes);
    config = history?.points?.length > 1
      ? buildRateLimitBars(stats, minutes)
      : buildRateLimitHistoryPending(stats, minutes);
  }

  elements.activityTitle.textContent = currentChartMode === "tokens" ? t("activityTrend", { days: stats.settings?.chartDays || currentSettings.chartDays }) : t("quotaConsumption");
  elements.chartSourceLabel.textContent = config.sourceLabel;
  renderChartHelp();
  elements.chartInterpretation.textContent = config.interpretation;
  renderChartLegend(config.legend);
  renderUsageBarChart(config);
}

function renderChartHelp() {
  const descriptionKey = currentChartMode === "tokens"
    ? "chartHelpTokens"
    : currentChartMode === "five-hour"
      ? "chartHelpFiveHour"
      : "chartHelpWeekly";
  elements.chartHelpButton.setAttribute("aria-label", t("chartHelp"));
  elements.chartHelpButton.setAttribute("title", t("chartHelp"));
  elements.chartHelpTitle.textContent = t("chartHelpTitle");
  elements.chartHelpText.textContent = `${t(descriptionKey)} ${t("chartHelpColor")}`;
}

function setChartHelpOpen(open) {
  const shouldOpen = Boolean(open);
  elements.chartHelpPopover.hidden = !shouldOpen;
  elements.chartHelpButton.setAttribute("aria-expanded", String(shouldOpen));
}

function showUsageChartTooltip(event, item) {
  const date = document.createElement("strong");
  date.textContent = item.label;

  const tokens = document.createElement("span");
  tokens.textContent = item.displayValue;

  const cost = document.createElement("span");
  cost.textContent = item.secondaryValue;

  elements.chartTooltip.replaceChildren(date, tokens, cost);
  elements.chartTooltip.hidden = false;
  moveChartTooltip(event);
}

function moveChartTooltip(event) {
  const rect = elements.dailyChart.getBoundingClientRect();
  const x = event.clientX - rect.left;
  const y = event.clientY - rect.top;
  elements.chartTooltip.style.left = `${Math.min(rect.width - 150, Math.max(8, x + 10))}px`;
  elements.chartTooltip.style.top = `${Math.max(8, y - 54)}px`;
}

function hideChartTooltip() {
  elements.chartTooltip.hidden = true;
}

function renderRecentThreads(threads) {
  elements.recentThreads.replaceChildren();

  if (!threads.length) {
    const empty = document.createElement("p");
    empty.className = "empty";
    empty.textContent = t("emptyRecentThreads");
    elements.recentThreads.append(empty);
    return;
  }

  for (const thread of threads) {
    const row = document.createElement("div");
    row.className = "thread-row";

    const time = document.createElement("span");
    time.className = "thread-time";
    time.textContent = formatTime(thread.updatedAt);

    const title = document.createElement("strong");
    title.className = "thread-title";
    title.textContent = thread.title || t("untitled");

    const workspace = document.createElement("span");
    workspace.className = "thread-workspace";
    const normalizedCwd = String(thread.cwd || "").replace(/[\\/]+$/, "");
    workspace.textContent = normalizedCwd.split(/[\\/]/).pop() || displaySourceName(thread.source);

    const model = document.createElement("span");
    model.className = "thread-model";
    model.textContent = thread.model || t("unknown");

    const tokens = document.createElement("span");
    tokens.className = "thread-token";
    tokens.textContent = formatCompact(thread.tokensUsed);

    row.append(time, title, workspace, model, tokens);
    elements.recentThreads.append(row);
  }
}

function formatInsightDay(value) {
  if (!value) return "-";
  const date = new Date(`${value}T12:00:00`);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(localeForLanguage(), {
    month: currentLanguage() === "zh" ? "long" : "short",
    day: "numeric"
  }).format(date);
}

function renderInsightSparkline(values = []) {
  const canvas = elements.insightSparkline;
  const rect = canvas.getBoundingClientRect();
  const width = Math.max(120, Math.round(rect.width || 180));
  const height = Math.max(24, Math.round(rect.height || 30));
  const scale = Math.max(1, window.devicePixelRatio || 1);
  canvas.width = Math.round(width * scale);
  canvas.height = Math.round(height * scale);
  const context = canvas.getContext("2d");
  context.clearRect(0, 0, canvas.width, canvas.height);

  const points = values.map((value) => Math.max(0, Number(value) || 0));
  if (points.length < 2 || !points.some((value) => value > 0)) return;

  const max = Math.max(...points, 1);
  const min = Math.min(...points);
  const range = Math.max(1, max - min);
  const inset = 2 * scale;
  context.beginPath();
  points.forEach((value, index) => {
    const x = inset + (index / (points.length - 1)) * (canvas.width - inset * 2);
    const y = inset + (1 - (value - min) / range) * (canvas.height - inset * 2);
    if (index === 0) context.moveTo(x, y);
    else context.lineTo(x, y);
  });
  context.lineWidth = 1.75 * scale;
  context.lineCap = "round";
  context.lineJoin = "round";
  context.strokeStyle = getComputedStyle(document.body).getPropertyValue("--bar-mid").trim() || "#85c1e9";
  context.stroke();
}

function renderUsageInsights(stats, fallbackDays = currentSettings.chartDays || 30) {
  const days = Array.isArray(stats?.dailySeries) ? stats.dailySeries : [];
  const chartDays = stats?.settings?.chartDays || fallbackDays;
  const recentValues = days.slice(-14).map((day) => Number(day.tokens) || 0);
  const recentSeven = recentValues.slice(-7).reduce((sum, value) => sum + value, 0);
  const previousSeven = recentValues.slice(-14, -7).reduce((sum, value) => sum + value, 0);
  const hasComparison = recentValues.length >= 14 && previousSeven > 0;
  const trendPercent = hasComparison ? ((recentSeven - previousSeven) / previousSeven) * 100 : null;

  elements.usageInsightsTitle.textContent = t("usageInsights");
  elements.usageInsightsMeta.textContent = t("usageInsightsMeta");
  elements.insightTrendLabel.textContent = t("insightTrendLabel");
  elements.insightTrendValue.textContent = trendPercent === null
    ? "-"
    : `${trendPercent >= 0 ? "+" : ""}${Math.round(trendPercent)}%`;
  elements.insightTrendValue.classList.toggle("negative", Number(trendPercent) < 0);
  elements.insightTrendNote.classList.remove("attention", "positive");
  if (trendPercent === null) {
    elements.insightTrendNote.textContent = t("insightTrendUnavailable");
  } else if (trendPercent >= 8) {
    elements.insightTrendNote.textContent = t("insightTrendUp");
    elements.insightTrendNote.classList.add("attention");
  } else if (trendPercent <= -8) {
    elements.insightTrendNote.textContent = t("insightTrendDown");
    elements.insightTrendNote.classList.add("positive");
  } else {
    elements.insightTrendNote.textContent = t("insightTrendSteady");
  }
  renderInsightSparkline(recentValues);

  const totalPeriodTokens = days.reduce((sum, day) => sum + (Number(day.tokens) || 0), 0);
  const peakDay = days.reduce((peak, day) => {
    return (Number(day.tokens) || 0) > (Number(peak?.tokens) || 0) ? day : peak;
  }, null);
  const dailyAverage = days.length ? totalPeriodTokens / days.length : 0;
  const peakRatio = dailyAverage > 0 ? (Number(peakDay?.tokens) || 0) / dailyAverage : 0;
  elements.insightPeakLabel.textContent = t("insightPeakLabel", { days: chartDays });
  elements.insightPeakValue.textContent = peakDay && Number(peakDay.tokens) > 0
    ? `${formatInsightDay(peakDay.date)} · ${formatCompact(peakDay.tokens)}`
    : "-";
  elements.insightPeakNote.textContent = peakRatio > 0
    ? t("insightPeakNote", { ratio: peakRatio.toFixed(1).replace(/\.0$/, "") })
    : t("insightNoData");

  const topWorkspace = Array.isArray(stats?.workspaces) ? stats.workspaces[0] : null;
  const totalTokens = Number(stats?.totals?.totalTokens) || 0;
  const workspaceShare = topWorkspace && totalTokens > 0
    ? Math.min(100, Math.max(0, (Number(topWorkspace.value) || 0) / totalTokens * 100))
    : 0;
  elements.insightWorkspaceLabel.textContent = t("insightWorkspaceLabel");
  elements.insightWorkspaceValue.textContent = topWorkspace
    ? `${topWorkspace.name} · ${Math.round(workspaceShare)}%`
    : "-";
  elements.insightWorkspaceNote.textContent = topWorkspace
    ? t(workspaceShare >= 50 ? "insightWorkspaceConcentrated" : "insightWorkspaceBalanced")
    : t("insightNoData");
  elements.insightWorkspaceValue.setAttribute("title", t("insightWorkspaceHint"));
  elements.insightWorkspaceNote.setAttribute("title", t("insightWorkspaceHint"));
}

function timeGreeting(date = new Date()) {
  const hour = date.getHours();
  if (currentLanguage() === "zh") {
    if (hour >= 5 && hour < 9) return "早上好";
    if (hour >= 9 && hour < 12) return "上午好";
    if (hour >= 12 && hour < 14) return "中午好";
    if (hour >= 14 && hour < 18) return "下午好";
    if (hour >= 18 && hour < 23) return "晚上好";
    return "夜深了";
  }
  if (hour >= 5 && hour < 12) return "Good morning";
  if (hour >= 12 && hour < 18) return "Good afternoon";
  if (hour >= 18 && hour < 23) return "Good evening";
  return "It's getting late";
}

function friendlyStatus(message) {
  return `${timeGreeting()}${currentLanguage() === "zh" ? "，" : ". "}${message}`;
}

function renderBriefMeta(stats) {
  const provider = PROVIDERS[currentSettings.activeProvider] || PROVIDERS.codex;
  const windows = availableRateLimitWindows(stats?.rateLimits);
  const official = currentSettings.activeProvider === "codex" && stats?.rateLimits?.planType !== "local_estimate";
  const allSafe = windows.length > 0 && windows.every((window) => Number(window.remainingPercent) >= 30);

  setProviderImage(elements.headerProviderLogo, currentSettings.activeProvider);
  elements.headerProviderLabel.textContent = provider.label;
  elements.quotaSourceBadge.textContent = official ? t("officialQuota") : t("localEstimate");
  elements.activityMeta.textContent = `${formatNumber(stats?.latestThreads?.length || 0)} ${currentLanguage() === "zh" ? "条本地记录" : "local records"}`;

  if (currentSettings.activeProvider === "chatgpt") {
    elements.statusHeading.textContent = friendlyStatus(currentLanguage() === "zh" ? "近期活动已经同步" : "Recent activity is synced");
    elements.quotaSourceBadge.textContent = t("localActivityEstimate");
  } else if (!windows.length) {
    elements.statusHeading.textContent = friendlyStatus(currentLanguage() === "zh" ? "额度数据还在同步" : "Quota data is still syncing");
  } else if (allSafe) {
    elements.statusHeading.textContent = friendlyStatus(
      currentLanguage() === "zh"
        ? windows.length > 1
          ? "当前两个额度窗口均安全"
          : "当前额度窗口状态安全"
        : windows.length > 1
          ? "Both quota windows are in good shape"
          : "The current quota window is in good shape"
    );
  } else {
    elements.statusHeading.textContent = friendlyStatus(
      currentLanguage() === "zh" ? "有额度窗口需要关注" : "A quota window needs attention"
    );
  }
}

function renderStats(stats) {
  clearSkeletons();
  lastStats = stats;
  const chartDays = stats.settings?.chartDays || currentSettings.chartDays;
  const mainLimit = primaryRateLimit(stats);
  const mainActivity = primaryActivityWindow(stats);
  const isChatgpt = currentSettings.activeProvider === "chatgpt";
  const provider = PROVIDERS[currentSettings.activeProvider] || PROVIDERS.codex;
  renderError(stats.error);

  elements.overviewProvider.textContent = provider.label;
  setProviderImage(elements.overviewProviderLogo, currentSettings.activeProvider);
  setProviderImage(elements.headerProviderLogo, currentSettings.activeProvider);
  elements.headerProviderLabel.textContent = provider.label;
  elements.overviewAccountName.textContent = stats.account?.displayName || provider.label;
  elements.overviewAccountPlan.textContent = stats.account?.planLabel || provider.label;
  applyProviderEstimateText(currentSettings.activeProvider);
  elements.periodTokensLabel.textContent = t("periodTokens", { days: chartDays });
  elements.periodUsageMeta.dataset.periodLabel = t("periodAccumulated", { days: chartDays });
  elements.activityTitle.textContent = t("activityTrend", { days: chartDays });
  elements.overviewPeriodLabel.textContent = t("daysPeriod", { days: chartDays });
  renderCostMetricLabels(chartDays);
  renderCostMetricValues(stats);
  elements.periodTokensContext.textContent = t("periodTokenContext", {
    total: formatCompact(stats.totals.totalTokens),
    latest: formatCompact(stats.featured.latestTokenUsage)
  });
  elements.periodUsageMeta.textContent = t("localRecordsOnly");
  elements.sidebarUsageLabel.textContent = isChatgpt ? t("recentActivity") : t("remainingUsage");
  elements.sidebarRemainingUsage.textContent = isChatgpt
    ? mainActivity
      ? t("activityCount", { count: formatNumber(mainActivity.count) })
      : "-"
    : mainLimit
      ? formatPercent(mainLimit.remainingPercent)
      : "-";
  elements.sidebarPeriodMeta.textContent = isChatgpt
    ? mainActivity
      ? formatLimitLabel(mainActivity)
      : t("waitingForLogs", { provider: provider.label })
    : mainLimit
      ? formatLimitMeta(mainLimit)
      : t("waitingForLogs", { provider: provider.label });
  elements.periodTokens.textContent = formatCompact(stats.featured.periodTokens);
  elements.todayTokens.textContent = formatCompact(stats.featured.todayTokens);
  updateTodayTokensMeter(statsRatioPercent(stats.featured.todayTokens, stats.featured.periodTokens));

  elements.threadsTotal.textContent = formatCompact(stats.totals.threads);
  elements.threadsActive.textContent = formatCompact(stats.totals.activeThreads);
  elements.tokensTotal.textContent = formatCompact(stats.totals.totalTokens);
  elements.updatedThisWeek.textContent = formatCompact(stats.totals.updatedThisWeek);
  elements.lastUpdated.textContent = t("updatedAt", { date: formatDate(stats.generatedAt) });
  elements.accountInitials.textContent = stats.account?.initials || provider.initials;
  elements.accountName.textContent = stats.account?.displayName || provider.label;
  elements.accountPlan.textContent = stats.account?.planLabel || provider.label;

  if (isChatgpt) {
    renderActivity(stats.activity);
  } else {
    renderRateLimits(stats.rateLimits);
  }
  renderUsageChart(stats);
  renderBriefMeta(stats);
  renderUsageInsights(stats);
  renderRankList(elements.modelList, stats.models, { compact: true });
  renderRankList(elements.sourceList, stats.sources, { formatName: displaySourceName });
  renderRankList(elements.workspaceList, stats.workspaces, { compact: true });
  renderRecentThreads(stats.latestThreads);
}

async function refreshStats(options = {}) {
  const { force = false, preferCache = false, showLoading = true } = options;
  const providerId = currentSettings.activeProvider;
  const requestId = latestStatsRequestId + 1;
  latestStatsRequestId = requestId;

  if (!force) {
    const cached = cachedStats(providerId);
    if (cached) {
      renderStats(cached);
      if (!preferCache) {
        return;
      }
    } else {
      renderStatsSkeleton(providerId);
    }
  } else {
    renderStatsSkeleton(providerId);
  }

  if (showLoading) {
    setLoading(true);
  }
  try {
    const stats = await aiUsage.getStats(providerId);
    if (requestId !== latestStatsRequestId || providerId !== currentSettings.activeProvider) {
      return;
    }
    cacheStats(providerId, stats);
    renderStats(stats);
    await refreshScanDiagnostics();
  } catch (error) {
    if (requestId !== latestStatsRequestId) {
      return;
    }
    const provider = PROVIDERS[providerId] || PROVIDERS.codex;
    renderError(error.message || t("readStatsError", { provider: provider.label }));
  } finally {
    if (requestId === latestStatsRequestId && showLoading) {
      setLoading(false);
    }
  }
}

async function rebuildCurrentProviderCache() {
  if (cacheRebuildInProgress) {
    return;
  }

  const providerId = currentSettings.activeProvider;
  cacheRebuildInProgress = true;
  cacheRebuildResult = null;
  latestStatsRequestId += 1;
  setLoading(true);
  renderScanDiagnostics();

  try {
    const stats = await aiUsage.rebuildStatsCache(providerId);
    cacheStats(providerId, stats);
    if (providerId === currentSettings.activeProvider) {
      renderStats(stats);
    }
    const diagnosticsLoaded = await refreshScanDiagnostics();
    const diagnostics = diagnosticsLoaded ? scanDiagnosticsForProvider(providerId) : null;
    cacheRebuildResult = {
      provider: providerId,
      error: "",
      problem: scanDiagnosticsProblem(diagnostics)
    };
  } catch (error) {
    cacheRebuildResult = {
      provider: providerId,
      error: error.message || t("cacheRebuildError")
    };
  } finally {
    cacheRebuildInProgress = false;
    setLoading(false);
    renderScanDiagnostics();
    resetAutoRefreshTimer();
  }
}

async function checkForUpdates(options = {}) {
  if (updateCheckInProgress) {
    return;
  }

  const { manual = false } = options;
  updateCheckInProgress = true;
  updateCheckError = "";
  renderUpdateSurfaces();

  try {
    updateInfo = await aiUsage.checkUpdate();
    if (manual && updateInfo.supported && updateInfo.available) {
      openUpdateDialog();
    }
  } catch (error) {
    if (manual) {
      updateInfo = { ...updateInfo, available: false, version: null };
      updateCheckError = error.message || t("updateCheckFailed");
    } else {
      updateInfo = { supported: false, available: false, currentVersion: null, version: null };
    }
  } finally {
    updateCheckInProgress = false;
    renderUpdateSurfaces();
  }
}

async function installAvailableUpdate() {
  if (!updateInfo.supported || !updateInfo.available || updateInstalling) {
    return;
  }

  updateInstalling = true;
  renderUpdateSurfaces();
  setLoading(currentLoading);

  try {
    await aiUsage.installUpdate();
  } catch (error) {
    renderError(error.message || t("installUpdateError"));
    updateInstalling = false;
    renderUpdateSurfaces();
    setLoading(currentLoading);
  }
}

async function saveSettings(nextSettings, shouldRefresh = true) {
  const languageChanged = Object.prototype.hasOwnProperty.call(nextSettings, "language");
  const normalizedNextSettings = { ...nextSettings };
  if (Object.prototype.hasOwnProperty.call(normalizedNextSettings, "chartDays")) {
    normalizedNextSettings.chartDays = normalizeChartDays(
      normalizedNextSettings.chartDays,
      normalizeChartDays(currentSettings.chartDays)
    );
  }
  if (Object.prototype.hasOwnProperty.call(normalizedNextSettings, "autoRefreshMinutes")) {
    normalizedNextSettings.autoRefreshMinutes = normalizeAutoRefreshMinutes(
      normalizedNextSettings.autoRefreshMinutes,
      normalizeAutoRefreshMinutes(currentSettings.autoRefreshMinutes)
    );
  }
  if (Object.prototype.hasOwnProperty.call(normalizedNextSettings, "autoRefreshEnabled")) {
    normalizedNextSettings.autoRefreshEnabled = Boolean(normalizedNextSettings.autoRefreshEnabled);
  }
  currentSettings = await aiUsage.updateSettings({
    ...currentSettings,
    ...normalizedNextSettings
  });
  if (languageChanged) {
    await aiUsage.syncTrayLanguage?.(currentLanguage());
  }
  settingsSavedAt = new Date();
  renderSettings();
  if (languageChanged && lastStats) {
    renderStats(lastStats);
  }
  if (shouldRefresh) {
    await refreshStats({ preferCache: true });
  }
  resetAutoRefreshTimer();
}

async function chooseHome(providerId) {
  setLoading(true);
  try {
    const result = await aiUsage.chooseHome(providerId);
    if (result) {
      currentSettings = result.settings;
      renderSettings();
      cacheStats(providerId, result.stats);
      renderStats(result.stats);
      await refreshScanDiagnostics();
      resetAutoRefreshTimer();
    }
  } catch (error) {
    const provider = PROVIDERS[providerId] || PROVIDERS.codex;
    renderError(error.message || t("switchHomeError", { provider: provider.label }));
  } finally {
    setLoading(false);
  }
}

function shouldStartWindowDrag(event) {
  if (event.button !== 0) return false;
  const target = event.target instanceof Element ? event.target : event.target?.parentElement;
  if (!target?.closest("[data-tauri-drag-region]")) return false;
  return !target.closest("button, input, select, textarea, a, [role='button']");
}

function startWindowDrag(event) {
  if (!shouldStartWindowDrag(event)) return;
  event.preventDefault();
  aiUsage.startWindowDrag().catch(() => {});
}

function openProjectLink(event, url) {
  event.preventDefault();
  aiUsage.openExternal(url).catch(() => {
    window.open(url, "_blank", "noopener,noreferrer");
  });
}

async function focusProviderByDirection(currentProvider, direction) {
  const visibleButtons = elements.providerButtons.filter((button) => !button.hidden && isProviderEnabled(button.dataset.provider));
  if (!visibleButtons.length) return;

  const currentIndex = Math.max(
    0,
    visibleButtons.findIndex((button) => button.dataset.provider === currentProvider)
  );
  const nextIndex = (currentIndex + direction + visibleButtons.length) % visibleButtons.length;
  const nextButton = visibleButtons[nextIndex];
  nextButton.focus();
  if (nextButton.dataset.provider !== currentSettings.activeProvider) {
    await saveSettings({ activeProvider: nextButton.dataset.provider });
  }
}

document.addEventListener("pointerdown", startWindowDrag, true);
elements.updateButton.addEventListener("click", openUpdateDialog);
elements.checkUpdateButton.addEventListener("click", () => checkForUpdates({ manual: true }));
elements.settingsInstallUpdateButton.addEventListener("click", openUpdateDialog);
elements.closeUpdateDialogButton.addEventListener("click", closeUpdateDialog);
elements.postponeUpdateButton.addEventListener("click", closeUpdateDialog);
elements.installUpdateButton.addEventListener("click", installAvailableUpdate);
elements.updateDialog.addEventListener("click", (event) => {
  if (event.target === elements.updateDialog) {
    closeUpdateDialog();
  }
});
elements.refreshButton.addEventListener("click", async () => {
  stopAutoRefreshTimer();
  await refreshStats({ force: true });
  resetAutoRefreshTimer();
});
elements.providerMenuButton.addEventListener("click", () => {
  setProviderMenuOpen(elements.sidebarProviderSection.hidden);
});
elements.chartHelpButton.addEventListener("click", (event) => {
  event.stopPropagation();
  setChartHelpOpen(elements.chartHelpPopover.hidden);
});
elements.chartHelpPopover.addEventListener("click", (event) => event.stopPropagation());
document.addEventListener("click", () => setChartHelpOpen(false));
elements.settingsButton.addEventListener("click", () => setView("settings"));
elements.homeButton.addEventListener("click", () => setHomeSection("summary"));
elements.trendButton.addEventListener("click", () => setHomeSection("trend"));
elements.activityButton.addEventListener("click", () => setHomeSection("activity"));
elements.settingsBackButton.addEventListener("click", () => setHomeSection("summary"));
elements.overviewPeriod.addEventListener("click", async () => {
  const presets = [7, 30, 90];
  const currentIndex = presets.indexOf(Number(currentSettings.chartDays));
  const nextDays = presets[(currentIndex + 1 + presets.length) % presets.length];
  await saveSettings({ chartDays: nextDays });
});
elements.repositoryLink.addEventListener("click", (event) => openProjectLink(event, REPOSITORY_URL));
elements.issueLink.addEventListener("click", (event) => openProjectLink(event, ISSUE_URL));
elements.rebuildCacheButton.addEventListener("click", rebuildCurrentProviderCache);
elements.chooseCodexHomeButton.addEventListener("click", () => chooseHome("codex"));
elements.chooseClaudeHomeButton.addEventListener("click", () => chooseHome("claude"));
elements.chooseCopilotHomeButton.addEventListener("click", () => chooseHome("copilot"));
elements.chooseCursorHomeButton.addEventListener("click", () => chooseHome("cursor"));
elements.chooseChatgptHomeButton.addEventListener("click", () => chooseHome("chatgpt"));
for (const button of elements.providerButtons) {
  button.addEventListener("click", async () => {
    if (!isProviderEnabled(button.dataset.provider)) return;
    currentChartMode = "tokens";
    setProviderMenuOpen(false);
    await saveSettings({ activeProvider: button.dataset.provider });
  });
  button.addEventListener("keydown", async (event) => {
    const directionByKey = {
      ArrowDown: 1,
      ArrowRight: 1,
      ArrowUp: -1,
      ArrowLeft: -1
    };
    const direction = directionByKey[event.key];
    if (!direction) return;
    event.preventDefault();
    await focusProviderByDirection(button.dataset.provider, direction);
  });
}
for (const button of elements.chartModeButtons) {
  button.addEventListener("click", () => {
    if (button.disabled || !lastStats) return;
    currentChartMode = button.dataset.chartMode;
    renderUsageChart(lastStats);
  });
  button.addEventListener("keydown", (event) => {
    if (!lastStats || !["ArrowLeft", "ArrowRight"].includes(event.key)) return;
    event.preventDefault();
    const enabled = elements.chartModeButtons.filter((candidate) => !candidate.disabled);
    const currentIndex = enabled.indexOf(button);
    const direction = event.key === "ArrowRight" ? 1 : -1;
    const next = enabled[(currentIndex + direction + enabled.length) % enabled.length];
    next.focus();
    next.click();
  });
}
document.addEventListener("click", (event) => {
  const target = event.target instanceof Element ? event.target : event.target?.parentElement;
  if (!target?.closest(".provider-switcher")) {
    setProviderMenuOpen(false);
  }
});
for (const input of elements.enabledProviderInputs) {
  input.addEventListener("change", async () => {
    const previousActiveProvider = currentSettings.activeProvider;
    const nextEnabledProviders = elements.enabledProviderInputs
      .filter((providerInput) => providerInput.checked)
      .map((providerInput) => providerInput.dataset.enabledProvider);

    if (!nextEnabledProviders.length) {
      input.checked = true;
      elements.settingsStatus.textContent = t("atLeastOneProvider");
      return;
    }

    const nextSettings = { enabledProviders: nextEnabledProviders };
    if (!nextEnabledProviders.includes(currentSettings.activeProvider)) {
      nextSettings.activeProvider = nextEnabledProviders[0];
    }
    await saveSettings(nextSettings, false);
    if (currentSettings.activeProvider !== previousActiveProvider) {
      await refreshStats({ preferCache: true, showLoading: false });
    }
  });
}
for (const button of elements.settingsNavButtons) {
  button.addEventListener("click", () => {
    const sectionId = button.dataset.settingsSection;
    const section = document.getElementById(sectionId);
    if (!section) return;
    clearSettingsSearch();
    activateSettingsNav(sectionId, { scroll: true });
  });
}
elements.settingsProvidersTab.addEventListener("click", () => {
  const preferredProvider = elements.settingsProviderNavButtons.find(
    (button) => button.dataset.settingsProvider === currentSettings.activeProvider
  );
  const fallbackProvider = elements.settingsProviderNavButtons[0];
  const sectionId = preferredProvider?.dataset.settingsSection || fallbackProvider?.dataset.settingsSection;
  if (!sectionId) return;
  clearSettingsSearch();
  activateSettingsNav(sectionId, { scroll: true });
});
elements.settingsSearchInput.addEventListener("input", updateSettingsSectionVisibility);
elements.themeSelect.addEventListener("change", async () => {
  await saveSettings({ theme: elements.themeSelect.value }, false);
});
elements.languageSelect.addEventListener("change", async () => {
  await saveSettings({ language: elements.languageSelect.value }, false);
});
elements.autoRefreshEnabledInput.addEventListener("change", async () => {
  await saveSettings({ autoRefreshEnabled: elements.autoRefreshEnabledInput.checked }, false);
});
elements.autoRefreshInput.addEventListener("change", async () => {
  const autoRefreshMinutes = normalizeAutoRefreshMinutes(
    elements.autoRefreshInput.value,
    normalizeAutoRefreshMinutes(currentSettings.autoRefreshMinutes)
  );
  elements.autoRefreshInput.value = autoRefreshMinutes;
  await saveSettings({ autoRefreshMinutes }, false);
});
for (const button of elements.accentButtons) {
  button.addEventListener("click", async () => {
    await saveSettings({ accentColor: button.dataset.accent }, false);
  });
}
for (const button of elements.periodButtons) {
  button.addEventListener("click", async () => {
    await saveSettings({ chartDays: Number(button.dataset.days) });
  });
}
elements.chartDaysInput.addEventListener("change", async () => {
  const chartDays = normalizeChartDays(elements.chartDaysInput.value, normalizeChartDays(currentSettings.chartDays));
  elements.chartDaysInput.value = chartDays;
  await saveSettings({ chartDays });
});
window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
  applyTheme(currentSettings.theme);
});
window.addEventListener("languagechange", () => {
  if (currentSettings.language !== "auto") return;
  Promise.resolve(aiUsage.syncTrayLanguage?.(currentLanguage())).catch(() => {});
  renderSettings();
  if (lastStats) {
    renderStats(lastStats);
  }
});
window.addEventListener("keydown", (event) => {
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "f" && currentView === "settings") {
    event.preventDefault();
    elements.settingsSearchInput.focus();
    elements.settingsSearchInput.select();
    return;
  }
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k" && currentView === "home") {
    event.preventDefault();
    setProviderMenuOpen(elements.sidebarProviderSection.hidden);
    if (!elements.sidebarProviderSection.hidden) {
      elements.providerButtons.find((button) => !button.hidden)?.focus();
    }
    return;
  }
  if (event.key === "Escape" && currentView === "settings" && elements.settingsSearchInput.value) {
    clearSettingsSearch();
    updateSettingsSectionVisibility();
    elements.settingsSearchInput.focus();
  } else if (event.key === "Escape" && !elements.chartHelpPopover.hidden) {
    setChartHelpOpen(false);
    elements.chartHelpButton.focus();
  } else if (event.key === "Escape" && !elements.updateDialog.hidden) {
    closeUpdateDialog();
  } else if (event.key === "Escape" && !elements.sidebarProviderSection.hidden) {
    setProviderMenuOpen(false);
    elements.providerMenuButton.focus();
  }
});

async function boot() {
  currentSettings = await aiUsage.getSettings();
  await aiUsage.syncTrayLanguage?.(currentLanguage());
  renderSettings();
  resetAutoRefreshTimer();
  setView(currentView);
  await Promise.all([refreshStats({ preferCache: true }), checkForUpdates()]);
}

boot();
