<template>
  <header class="app-header" data-tauri-drag-region>
    <div class="header-leading" data-tauri-drag-region>
      <div class="brand" data-tauri-drag-region>
        <span class="brand-mark" aria-hidden="true">
          <img class="brand-logo brand-logo-light" src="../assets/app-icon.svg" alt="" />
          <img class="brand-logo brand-logo-dark" src="../assets/app-icon-dark.svg" alt="" />
        </span>
        <strong data-tauri-drag-region>Dial</strong>
        <span id="brandSubtitle" class="sr-only">Usage monitor</span>
      </div>

      <div class="provider-switcher">
        <button id="providerMenuButton" class="provider-menu-button" type="button" aria-haspopup="true" aria-expanded="false">
          <span class="provider-option-logo" aria-hidden="true">
            <img id="headerProviderLogo" src="../assets/provider-codex-light.svg" data-provider-logo="codex" alt="" />
          </span>
          <span id="headerProviderLabel">Codex</span>
          <span class="ui-icon icon-chevron-down provider-menu-hint" aria-hidden="true"></span>
        </button>
        <section id="sidebarProviderSection" class="sidebar-provider" aria-label="AI 服务" hidden>
          <span id="sidebarProviderLabel">AI 服务</span>
          <div id="providerOptions" class="provider-options compact" role="radiogroup" aria-label="AI 服务">
            <button class="provider-option active" type="button" role="radio" data-provider="codex" aria-checked="true">
              <span class="provider-option-logo" aria-hidden="true"><img src="../assets/provider-codex-light.svg" data-provider-logo="codex" alt="" /></span>
              <span>Codex</span>
            </button>
            <button class="provider-option" type="button" role="radio" data-provider="claude" aria-checked="false">
              <span class="provider-option-logo" aria-hidden="true"><img src="../assets/provider-claude.svg" data-provider-logo="claude" alt="" /></span>
              <span>Claude Code</span>
            </button>
            <button class="provider-option" type="button" role="radio" data-provider="copilot" aria-checked="false">
              <span class="provider-option-logo" aria-hidden="true"><img src="../assets/provider-copilot.svg" data-provider-logo="copilot" alt="" /></span>
              <span>GitHub Copilot</span>
            </button>
            <button class="provider-option" type="button" role="radio" data-provider="cursor" aria-checked="false">
              <span class="provider-option-logo" aria-hidden="true"><img src="../assets/provider-cursor.svg" data-provider-logo="cursor" alt="" /></span>
              <span>Cursor</span>
            </button>
            <button class="provider-option" type="button" role="radio" data-provider="chatgpt" aria-checked="false">
              <span class="provider-option-logo" aria-hidden="true"><img src="../assets/provider-chatgpt.svg" data-provider-logo="chatgpt" alt="" /></span>
              <span>ChatGPT</span>
            </button>
          </div>
          <section class="account-pill" aria-label="AI account">
            <div id="accountInitials" class="account-avatar">CD</div>
            <div><strong id="accountName">Codex</strong><span id="accountPlan">Codex</span></div>
          </section>
        </section>
      </div>
    </div>

    <nav id="primaryNav" class="primary-nav" aria-label="Primary navigation">
      <button id="homeButton" class="nav-button active" type="button">摘要</button>
      <button id="trendButton" class="nav-button" type="button">趋势</button>
      <button id="activityButton" class="nav-button" type="button">活动</button>
    </nav>

    <div class="topbar-actions">
      <span id="sourceShortcutButton" class="source-shortcut">⌘K 切换数据源</span>
      <button id="overviewPeriod" class="text-button period-button" type="button">
        <span class="ui-icon icon-calendar" aria-hidden="true"></span>
        <span id="overviewPeriodLabel">近 30 天</span>
        <span class="ui-icon icon-chevron-down period-chevron" aria-hidden="true"></span>
      </button>
      <button id="updateButton" class="update-button" type="button" hidden>更新</button>
      <span id="autoRefreshCountdown" class="refresh-countdown" hidden>-</span>
      <button id="refreshButton" class="icon-button header-icon-button" type="button" aria-label="刷新" title="刷新">
        <span class="ui-icon icon-refresh" aria-hidden="true"></span>
      </button>
      <button id="settingsButton" class="icon-button header-icon-button" type="button" aria-label="设置" title="设置">
        <span class="ui-icon icon-settings" aria-hidden="true"></span>
      </button>
    </div>
    <p id="viewEyebrow" class="sr-only">Codex usage</p>
    <h1 id="viewTitle" class="sr-only">概览</h1>
  </header>
</template>
