<template>
  <section id="homeView" class="usage-brief">
    <section class="usage-console" aria-label="Featured usage summary">
      <h2 id="statusHeading">正在读取额度状态</h2>
      <p class="trust-line">
        <span class="ui-icon icon-shield-check" aria-hidden="true"></span>
        <span id="quotaSourceBadge" class="source-badge">官方限额</span>
        <span id="rateLimitUpdated">-</span>
      </p>
      <div class="overview-status sr-only" aria-hidden="true">
        <img id="overviewProviderLogo" src="../assets/provider-codex-light.svg" data-provider-logo="codex" alt="" />
        <span id="overviewSourceLabel">数据源</span>
        <strong id="overviewProvider">Codex</strong>
        <span id="overviewEstimateLabel">本地日志估算</span>
        <span id="overviewAccountName">Codex</span>
        <strong id="overviewAccountPlan">Codex</strong>
      </div>
    </section>

    <section class="quota-panel" aria-labelledby="rateLimitTitle">
      <h2 id="rateLimitTitle" class="sr-only">额度窗口</h2>
      <div id="rateLimitList" class="limit-list"></div>
    </section>

    <section class="brief-grid">
      <article id="usageChartPanel" class="panel chart-panel">
        <div class="chart-heading">
          <div class="chart-copy">
            <div class="chart-title-line">
              <h2 id="activityTitle">额度消耗</h2>
              <button
                id="chartHelpButton"
                class="hint-button"
                type="button"
                aria-label="图表说明"
                aria-expanded="false"
                aria-controls="chartHelpPopover"
              >
                <span class="ui-icon icon-info-circle" aria-hidden="true"></span>
              </button>
            </div>
            <span id="chartSourceLabel">官方额度快照</span>
            <section id="chartHelpPopover" class="chart-help-popover" role="dialog" aria-labelledby="chartHelpTitle" hidden>
              <strong id="chartHelpTitle">如何阅读图表</strong>
              <p id="chartHelpText">柱高表示数据量，颜色越深表示相对用量越高。</p>
            </section>
          </div>
          <div id="chartModeTabs" class="chart-mode-tabs" role="tablist" aria-label="图表模式">
            <button type="button" role="tab" data-chart-mode="five-hour" aria-selected="false" hidden>5 小时</button>
            <button type="button" role="tab" data-chart-mode="weekly" aria-selected="false" hidden>每周</button>
            <button type="button" role="tab" data-chart-mode="tokens" aria-selected="true">Token</button>
          </div>
        </div>
        <div id="chartLegend" class="chart-legend"></div>
        <div class="chart-wrap">
          <div id="dailyChart" class="daily-chart" role="img" aria-label="用量柱状图"></div>
          <div id="chartTooltip" class="chart-tooltip" hidden></div>
        </div>
        <p id="chartInterpretation" class="chart-interpretation">-</p>
        <span id="lastUpdated" class="sr-only">-</span>
      </article>

      <article class="panel today-summary" aria-labelledby="todaySummaryTitle">
        <h2 id="todaySummaryTitle">今日摘要</h2>
        <div class="summary-item">
          <span id="todayTokensLabel">今日 token 用量</span>
          <strong id="todayTokens">-</strong>
          <small>本机日志</small>
        </div>
        <div class="summary-item">
          <span id="threadsTotalLabel">累计会话</span>
          <strong id="threadsTotal">-</strong>
          <small id="threadsTotalMeta">全部本地记录</small>
        </div>
        <div class="summary-item estimate-summary">
          <span id="todayCostLabel">费用估算</span>
          <strong id="todayCost">-</strong>
          <small>非官方账单</small>
        </div>
      </article>
    </section>

    <section class="usage-insights" aria-labelledby="usageInsightsTitle">
      <div class="insights-heading">
        <h2 id="usageInsightsTitle">本期洞察</h2>
        <span id="usageInsightsMeta">基于本地记录</span>
      </div>
      <div class="insight-list">
        <div class="insight-row">
          <span class="insight-icon" aria-hidden="true"><span class="ui-icon icon-chart-line"></span></span>
          <p class="insight-primary"><span id="insightTrendLabel">近 7 天 token 比前 7 天</span><strong id="insightTrendValue">-</strong></p>
          <canvas id="insightSparkline" class="insight-sparkline" width="180" height="30" aria-hidden="true"></canvas>
          <span id="insightTrendNote" class="insight-note">-</span>
        </div>
        <div class="insight-row">
          <span class="insight-icon" aria-hidden="true"><span class="ui-icon icon-calendar"></span></span>
          <p class="insight-primary"><span id="insightPeakLabel">用量最高日</span><strong id="insightPeakValue">-</strong></p>
          <span class="insight-spacer" aria-hidden="true"></span>
          <span id="insightPeakNote" class="insight-note">-</span>
        </div>
        <div class="insight-row">
          <span class="insight-icon" aria-hidden="true"><span class="ui-icon icon-chart-pie"></span></span>
          <p class="insight-primary"><span id="insightWorkspaceLabel">最大工作区</span><strong id="insightWorkspaceValue">-</strong></p>
          <span class="insight-spacer" aria-hidden="true"></span>
          <span id="insightWorkspaceNote" class="insight-note">-</span>
        </div>
      </div>
    </section>

    <section id="activitySection" class="recent-panel" aria-labelledby="recentThreadsTitle">
      <div class="section-heading">
        <h2 id="recentThreadsTitle">最近活动</h2>
        <span id="activityMeta">本地记录</span>
      </div>
      <div id="recentThreads" class="thread-list"></div>
    </section>

    <section class="diagnostic-data" hidden aria-hidden="true">
      <span id="periodTokensLabel">近 30 天 token 用量</span><strong id="periodTokens">-</strong>
      <p id="periodTokensContext">-</p><small id="periodUsageMeta">-</small>
      <span id="periodCostLabel">近 30 天费用</span><strong id="periodCost">-</strong>
      <div id="todayTokensMeter" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow="0"></div>
      <span id="threadsActiveLabel">活跃</span><strong id="threadsActive">-</strong>
      <span id="tokensTotalLabel">总 token</span><strong id="tokensTotal">-</strong>
      <span id="updatedThisWeekLabel">近 7 天更新</span><strong id="updatedThisWeek">-</strong>
      <h2 id="modelTitle">模型</h2><div id="modelList"></div>
      <h2 id="sourceTitle">运行来源</h2><div id="sourceList"></div>
      <h2 id="workspaceTitle">工作区</h2><div id="workspaceList"></div>
    </section>
    <section id="sidebarUsageSection" hidden aria-hidden="true">
      <span id="sidebarUsageLabel">剩余用量</span><strong id="sidebarRemainingUsage">-</strong><small id="sidebarPeriodMeta">近 30 天</small>
    </section>
  </section>
</template>
