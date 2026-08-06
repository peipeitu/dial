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
  openButton: document.getElementById("openButton")
};

const bundledAssets = window.__AI_USAGE_ASSETS__ || {};
const providerAssets = new Set(["codex", "claude", "copilot", "cursor", "chatgpt"]);
const themes = new Set(["light", "dark"]);
const accents = new Set(["blue", "turquoise", "green", "purple", "red", "orange", "graphite"]);

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
