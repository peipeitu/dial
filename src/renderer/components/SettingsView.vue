<template>
  <section id="settingsView" class="settings-view" hidden>
    <div class="settings-drag-region" data-tauri-drag-region aria-hidden="true"></div>
    <div class="settings-shell">
      <header class="settings-topbar" data-tauri-drag-region>
        <button id="settingsBackButton" class="settings-back" type="button">
          <span class="ui-icon icon-arrow-left" aria-hidden="true"></span>
          <span id="settingsBackLabel">返回应用</span>
        </button>
        <h1 id="settingsWindowTitle" data-tauri-drag-region>设置</h1>
        <span class="settings-save-status">
          <span class="ui-icon icon-check" aria-hidden="true"></span>
          <span id="settingsStatus">-</span>
        </span>
      </header>

      <div class="settings-category-bar">
        <span id="settingsPersonalLabel" class="sr-only" aria-hidden="true">个人</span>
        <nav class="settings-nav" aria-label="Settings sections">
          <button class="settings-nav-item active" type="button" data-settings-section="settingsGeneralSection">
            <span class="ui-icon icon-settings" aria-hidden="true"></span>
            <span id="settingsGeneralNavLabel">常规</span>
          </button>
          <button class="settings-nav-item" type="button" data-settings-section="settingsAppearanceSection">
            <span class="ui-icon icon-palette" aria-hidden="true"></span>
            <span id="settingsAppearanceNavLabel">个性化</span>
          </button>
          <button class="settings-nav-item" type="button" data-settings-section="settingsChartSection">
            <span class="ui-icon icon-chart-line" aria-hidden="true"></span>
            <span id="settingsChartNavLabel">图表</span>
          </button>
          <button class="settings-nav-item" type="button" data-settings-section="settingsDataSection">
            <span class="ui-icon icon-cpu" aria-hidden="true"></span>
            <span id="settingsDataNavLabel">数据与缓存</span>
          </button>
          <button class="settings-nav-item" type="button" data-settings-section="settingsUpdateSection">
            <span class="ui-icon icon-refresh" aria-hidden="true"></span>
            <span id="settingsUpdateNavLabel">更新</span>
          </button>
          <button id="settingsProvidersTab" class="settings-nav-item" type="button" data-settings-group="providers">
            <span class="ui-icon icon-cpu" aria-hidden="true"></span>
            <span id="settingsProvidersLabel">AI 服务</span>
          </button>
        </nav>

        <label class="settings-search">
          <span class="ui-icon icon-search" aria-hidden="true"></span>
          <input id="settingsSearchInput" type="search" placeholder="搜索设置" autocomplete="off" />
        </label>
      </div>

      <div class="settings-content">
        <header class="settings-content-header">
          <h2 id="settingsPanelTitle" class="settings-primary-heading">常规设置</h2>
          <p id="settingsSectionDescription">配置应用的基本行为和使用偏好。</p>
        </header>

        <div id="settingsProviderTabs" class="settings-provider-tabs" hidden>
          <button
            v-for="provider in providers"
            :key="provider.id"
            class="settings-provider-tab"
            type="button"
            :data-settings-section="provider.section"
            :data-settings-provider="provider.id"
          >
            <span class="settings-nav-icon provider-logo" aria-hidden="true">
              <img :src="provider.icon" :data-provider-logo="provider.id" alt="" />
            </span>
            <span :id="provider.navLabelId">{{ provider.label }}</span>
          </button>
        </div>

        <div class="settings-list">
          <section id="settingsGeneralSection" class="settings-section">
            <div class="settings-section-heading"><h3 id="settingsGeneralTitle">应用</h3></div>
            <div class="settings-group">
              <label class="setting-row">
                <span id="languageLabel">语言</span>
                <select id="languageSelect">
                  <option id="languageAutoOption" value="auto">跟随系统</option>
                  <option id="languageZhOption" value="zh">中文</option>
                  <option id="languageEnOption" value="en">English</option>
                </select>
              </label>
              <label class="setting-row">
                <span id="autoRefreshLabel">自动更新</span>
                <div class="auto-refresh-control">
                  <label class="toggle-item auto-refresh-toggle">
                    <input id="autoRefreshEnabledInput" type="checkbox" />
                    <span id="autoRefreshEnabledLabel">启用</span>
                  </label>
                  <label class="days-input">
                    <input id="autoRefreshInput" type="number" min="1" max="1440" step="1" />
                    <span id="autoRefreshSuffix">分钟</span>
                  </label>
                </div>
              </label>
            </div>
          </section>

          <section id="settingsAppearanceSection" class="settings-section">
            <div class="settings-section-heading"><h3 id="settingsAppearanceTitle">个性化</h3></div>
            <div class="settings-group">
              <label class="setting-row">
                <span id="themeLabel">主题</span>
                <select id="themeSelect">
                  <option id="themeSystemOption" value="system">跟随系统</option>
                  <option id="themeLightOption" value="light">浅色</option>
                  <option id="themeDarkOption" value="dark">深色</option>
                </select>
              </label>
              <label class="setting-row">
                <span id="accentLabel">主题色</span>
                <div id="accentOptions" class="accent-options" role="radiogroup" aria-label="主题色">
                  <button v-for="accent in accents" :key="accent" class="swatch" type="button" :data-accent="accent" :aria-label="accent"></button>
                </div>
              </label>
            </div>
          </section>

          <section id="settingsChartSection" class="settings-section">
            <div class="settings-section-heading"><h3 id="settingsChartTitle">使用体验</h3></div>
            <div class="settings-group">
              <label class="setting-row">
                <span id="chartPeriodLabel">图表周期</span>
                <div class="period-control">
                  <div id="periodPresets" class="period-presets" aria-label="图表周期快捷选择">
                    <button id="period7Button" type="button" data-days="7">一周</button>
                    <button id="period30Button" type="button" data-days="30">一个月</button>
                    <button id="period90Button" type="button" data-days="90">三个月</button>
                  </div>
                  <label class="days-input">
                    <input id="chartDaysInput" type="number" min="7" max="90" step="1" />
                    <span id="daysSuffix">天</span>
                  </label>
                </div>
              </label>
            </div>
          </section>

          <section id="settingsDataSection" class="settings-section">
            <div class="settings-section-heading"><h3 id="settingsDataTitle">数据与缓存</h3></div>
            <div class="settings-group">
              <div class="setting-row">
                <span id="scanProviderLabel">当前服务</span>
                <code id="scanProviderValue" class="setting-inline-value">Codex</code>
              </div>
              <div class="setting-row">
                <span id="latestScanLabel">最近扫描</span>
                <div class="setting-control scan-diagnostics-control">
                  <strong id="scanDiagnosticsSummary">刷新后显示</strong>
                  <small id="scanDiagnosticsMeta">-</small>
                </div>
              </div>
              <div class="setting-row">
                <span id="cacheManagementLabel">扫描缓存</span>
                <div class="setting-control cache-setting-control">
                  <span id="rebuildCacheStatus" class="setting-status-text" role="status" aria-live="polite">-</span>
                  <button id="rebuildCacheButton" class="secondary-button" type="button">重建缓存</button>
                </div>
              </div>
            </div>
          </section>

          <section id="settingsUpdateSection" class="settings-section">
            <div class="settings-section-heading"><h3 id="settingsUpdateTitle">更新</h3></div>
            <div class="settings-group">
              <div class="setting-row">
                <span id="currentVersionLabel">当前版本</span>
                <code id="currentVersionValue" class="setting-inline-value">-</code>
              </div>
              <div class="setting-row">
                <span id="latestVersionLabel">最新版本</span>
                <div class="setting-control update-setting-control">
                  <code id="latestVersionValue" class="setting-inline-value">-</code>
                  <span id="settingsUpdateStatus" class="setting-status-text">-</span>
                  <button id="checkUpdateButton" class="secondary-button" type="button">检查更新</button>
                  <button id="settingsInstallUpdateButton" type="button" hidden>立即更新</button>
                </div>
              </div>
            </div>
          </section>

          <div id="settingsProvidersContentTitle" class="sr-only" aria-hidden="true">AI 服务</div>

          <section v-for="provider in providers" :id="provider.section" :key="provider.id" class="settings-section">
            <div class="settings-section-heading"><h3 :id="provider.titleId">{{ provider.label }}</h3></div>
            <div class="settings-group">
              <div class="setting-row">
                <span data-provider-enabled-label>启用</span>
                <label class="toggle-item provider-toggle">
                  <input :id="provider.enableId" type="checkbox" :data-enabled-provider="provider.id" />
                  <span data-provider-toggle-label>启用</span>
                </label>
              </div>
              <label class="setting-row">
                <span :id="provider.homeLabelId">数据目录</span>
                <div class="setting-control folder-control">
                  <code :id="provider.homeValueId">-</code>
                  <button :id="provider.chooseId" class="secondary-button" type="button">选择目录</button>
                </div>
              </label>
            </div>
          </section>
        </div>

        <div id="settingsNoResults" class="settings-no-results" hidden>
          <span class="ui-icon icon-search" aria-hidden="true"></span>
          <strong id="settingsNoResultsTitle">未找到相关设置</strong>
        </div>

        <section class="project-links settings-links" aria-label="Project links">
          <a id="repositoryLink" href="https://github.com/peipeitu/dial" target="_blank" rel="noreferrer" aria-label="GitHub" title="GitHub">
            <span class="ui-icon icon-brand-github" aria-hidden="true"></span>
          </a>
          <a id="issueLink" href="https://github.com/peipeitu/dial/issues" target="_blank" rel="noreferrer" aria-label="Feedback" title="Feedback">
            <span class="ui-icon icon-message" aria-hidden="true"></span>
          </a>
        </section>
      </div>
    </div>
  </section>
</template>

<script setup>
import claudeIcon from "../assets/provider-claude.svg";
import chatgptIcon from "../assets/provider-chatgpt.svg";
import codexIcon from "../assets/provider-codex-light.svg";
import copilotIcon from "../assets/provider-copilot.svg";
import cursorIcon from "../assets/provider-cursor.svg";

const accents = ["blue", "turquoise", "green", "purple", "red", "orange", "graphite"];

const providers = [
  {
    id: "codex",
    label: "Codex",
    icon: codexIcon,
    section: "settingsCodexProviderSection",
    navLabelId: "settingsCodexNavLabel",
    titleId: "settingsCodexProviderTitle",
    enableId: "enableCodexInput",
    homeLabelId: "codexHomeLabel",
    homeValueId: "codexHomeValue",
    chooseId: "chooseCodexHomeButton"
  },
  {
    id: "claude",
    label: "Claude Code",
    icon: claudeIcon,
    section: "settingsClaudeProviderSection",
    navLabelId: "settingsClaudeNavLabel",
    titleId: "settingsClaudeProviderTitle",
    enableId: "enableClaudeInput",
    homeLabelId: "claudeHomeLabel",
    homeValueId: "claudeHomeValue",
    chooseId: "chooseClaudeHomeButton"
  },
  {
    id: "copilot",
    label: "GitHub Copilot",
    icon: copilotIcon,
    section: "settingsCopilotProviderSection",
    navLabelId: "settingsCopilotNavLabel",
    titleId: "settingsCopilotProviderTitle",
    enableId: "enableCopilotInput",
    homeLabelId: "copilotHomeLabel",
    homeValueId: "copilotHomeValue",
    chooseId: "chooseCopilotHomeButton"
  },
  {
    id: "cursor",
    label: "Cursor",
    icon: cursorIcon,
    section: "settingsCursorProviderSection",
    navLabelId: "settingsCursorNavLabel",
    titleId: "settingsCursorProviderTitle",
    enableId: "enableCursorInput",
    homeLabelId: "cursorHomeLabel",
    homeValueId: "cursorHomeValue",
    chooseId: "chooseCursorHomeButton"
  },
  {
    id: "chatgpt",
    label: "ChatGPT",
    icon: chatgptIcon,
    section: "settingsChatgptProviderSection",
    navLabelId: "settingsChatgptNavLabel",
    titleId: "settingsChatgptProviderTitle",
    enableId: "enableChatgptInput",
    homeLabelId: "chatgptHomeLabel",
    homeValueId: "chatgptHomeValue",
    chooseId: "chooseChatgptHomeButton"
  }
];
</script>
