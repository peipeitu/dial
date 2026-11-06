const tauriCore = window.__TAURI__?.core;
const tauriEvent = window.__TAURI__?.event;
const tauriWindow = window.__TAURI__?.window;

const elements = {
  providerIcon: document.getElementById("providerIcon"),
  providerName: document.getElementById("providerName"),
  usageLabel: document.getElementById("usageLabel"),
  usageValue: document.getElementById("usageValue"),
  usageTrack: document.getElementById("usageTrack"),
  usageFill: document.getElementById("usageFill"),
  usageMeta: document.getElementById("usageMeta"),
  usageSummary: document.getElementById("usageSummary"),
  quotaList: document.getElementById("quotaList"),
  openButton: document.getElementById("openButton")
};

const bundledAssets = window.__AI_USAGE_ASSETS__ || {};
const providerAssets = new Set(["codex", "claude", "copilot", "cursor", "chatgpt"]);
const themes = new Set(["light", "dark"]);
const accents = new Set(["blue", "turquoise", "green", "purple", "red", "orange", "graphite"]);
let countdowns = [];
let statusLanguage = "zh-CN";

function quotaLabel(minutes) {
  const zh = statusLanguage.startsWith("zh");
  if (minutes === 10080) return zh ? "每周" : "Weekly";
  for (const [size, cn, en] of [[10080, "周", "weeks"], [1440, "天", "days"], [60, "小时", "hours"]]) {
    if (minutes >= size && minutes % size === 0) return `${minutes / size} ${zh ? cn : en}`;
  }
  return `${minutes} ${zh ? "分钟" : "minutes"}`;
}

function resetCountdown(value, now = Date.now()) {
  const zh = statusLanguage.startsWith("zh");
  const reset = value ? new Date(value).getTime() : NaN;
  if (!Number.isFinite(reset)) return zh ? "更新时间未知" : "Reset time unavailable";
  const seconds = Math.max(0, Math.ceil((reset - now) / 1000));
  if (!seconds) return zh ? "即将更新" : "Updating soon";
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor(seconds % 86400 / 3600);
  const minutes = Math.floor(seconds % 3600 / 60);
  const secs = seconds % 60;
  const time = days ? (zh ? `${days}天 ${hours}小时` : `${days}d ${hours}h`)
    : hours ? (zh ? `${hours}小时 ${minutes}分` : `${hours}h ${minutes}m`)
      : minutes ? (zh ? `${minutes}分 ${secs}秒` : `${minutes}m ${secs}s`)
        : (zh ? `${secs}秒` : `${secs}s`);
  return zh ? `更新 ${time}` : `Updates in ${time}`;
}

function updateCountdowns() {
  for (const { node, resetsAt } of countdowns) node.textContent = resetCountdown(resetsAt);
}

function renderQuotas(windows) {
  elements.quotaList.replaceChildren();
  countdowns = [];
  const available = (Array.isArray(windows) ? windows : [])
    .filter(limit => Number(limit.windowMinutes) > 0 && normalizedPercent(limit.remainingPercent) !== null)
    .sort((a, b) => a.windowMinutes - b.windowMinutes);
  elements.usageSummary.hidden = available.length > 0;
  for (const limit of available) {
    const percent = normalizedPercent(limit.remainingPercent);
    const minutes = Number(limit.windowMinutes);
    const row = document.createElement("section");
    row.className = "quota-row";
    const heading = document.createElement("div");
    heading.className = "usage-heading";
    const label = document.createElement("span");
    label.textContent = quotaLabel(minutes);
    const value = document.createElement("strong");
    value.textContent = `${percent}% ${statusLanguage.startsWith("zh") ? "剩余" : "remaining"}`;
    heading.append(label, value);
    const meter = document.createElement("div");
    meter.className = "quota-meter";
    meter.setAttribute("role", "progressbar");
    meter.setAttribute("aria-label", `${label.textContent} · ${value.textContent}`);
    meter.setAttribute("aria-valuemin", "0");
    meter.setAttribute("aria-valuemax", "100");
    meter.setAttribute("aria-valuenow", String(percent));
    const count = minutes === 300 ? 5 : minutes === 10080 ? 7 : 10;
    for (let index = 0; index < count; index += 1) {
      const segment = document.createElement("span");
      const fill = document.createElement("i");
      fill.style.width = `${Math.max(0, Math.min(100, percent * count - index * 100))}%`;
      segment.append(fill);
      meter.append(segment);
    }
    const countdown = document.createElement("p");
    countdown.className = "quota-countdown";
    countdowns.push({ node: countdown, resetsAt: limit.resetsAt });
    row.append(heading, meter, countdown);
    elements.quotaList.append(row);
  }
  updateCountdowns();
}

function normalizedPercent(value) {
  if (value === null || value === undefined) return null;
  const percent = Number(value);
  if (!Number.isFinite(percent)) return null;
  return Math.min(100, Math.max(0, Math.round(percent)));
}

function renderStatus(status) {
  const provider = providerAssets.has(status?.provider) ? status.provider : "codex";
  const percent = normalizedPercent(status?.remainingPercent);
  const available = percent !== null;
  const theme = themes.has(status?.theme) ? status.theme : null;

  document.documentElement.lang = status?.language || "zh-CN";
  statusLanguage = document.documentElement.lang;
  if (theme) {
    document.body.dataset.theme = theme;
  } else {
    delete document.body.dataset.theme;
  }
  document.body.dataset.accent = accents.has(status?.accentColor) ? status.accentColor : "blue";

  elements.providerIcon.src = bundledAssets[provider] || `./assets/provider-${provider}.svg`;
  elements.providerName.textContent = status?.providerLabel || "Dial";
  elements.usageLabel.textContent = status?.usageLabel || "剩余用量";
  elements.openButton.textContent = status?.openLabel || "打开详情";
  elements.usageValue.textContent = available ? `${percent}%` : "--";
  elements.usageFill.style.width = available ? `${percent}%` : "0";
  elements.usageMeta.textContent = status?.statusLabel || (available ? "自动更新已关闭" : "暂无可用数据");
  elements.usageTrack.setAttribute("aria-valuenow", available ? String(percent) : "0");
  elements.usageTrack.setAttribute(
    "aria-valuetext",
    available ? `${elements.usageLabel.textContent} ${percent}%` : elements.usageMeta.textContent
  );
  renderQuotas(status?.windows);
}

async function openMainWindow() {
  elements.openButton.disabled = true;
  try {
    await tauriCore.invoke("open_main_window");
  } finally {
    elements.openButton.disabled = false;
  }
}

async function boot() {
  if (!tauriCore?.invoke) {
    elements.usageMeta.textContent = "Unable to connect";
    return;
  }

  if (tauriEvent?.listen) {
    try {
      await tauriEvent.listen("tray-status-updated", (event) => renderStatus(event.payload));
    } catch {
      // The initial command below can still provide a usable status snapshot.
    }
  }

  try {
    renderStatus(await tauriCore.invoke("get_tray_status"));
  } catch {
    elements.usageMeta.textContent = "Unable to read usage";
  }
}

elements.openButton.addEventListener("click", openMainWindow);
window.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    const currentWindow = tauriWindow?.getCurrentWindow?.();
    currentWindow?.hide?.();
  }
});

boot();
window.setInterval(updateCountdowns, 1000);
