<template>
  <div id="updateDialog" class="modal-backdrop" hidden>
    <section class="update-dialog" role="dialog" aria-modal="true" aria-labelledby="updateDialogTitle" aria-describedby="updateDialogSubtitle">
      <header class="update-dialog-header">
        <div>
          <span id="updateDialogEyebrow" class="eyebrow">可用更新</span>
          <h2 id="updateDialogTitle">发现新版本</h2>
          <p id="updateDialogSubtitle">安装后应用会自动重启。</p>
        </div>
        <button id="closeUpdateDialogButton" class="icon-button" type="button" aria-label="关闭" title="关闭">
          <span aria-hidden="true">×</span>
        </button>
      </header>
      <dl class="update-version-grid">
        <div><dt id="dialogCurrentVersionLabel">当前版本</dt><dd id="dialogCurrentVersion">-</dd></div>
        <div><dt id="dialogLatestVersionLabel">最新版本</dt><dd id="dialogLatestVersion">-</dd></div>
        <div><dt id="dialogPublishedAtLabel">发布时间</dt><dd id="dialogPublishedAt">-</dd></div>
      </dl>
      <section class="update-notes-section">
        <h3 id="updateNotesTitle">更新说明</h3>
        <pre id="updateNotes">-</pre>
      </section>
      <footer class="update-dialog-actions">
        <button id="postponeUpdateButton" class="secondary-button" type="button">稍后</button>
        <button id="installUpdateButton" type="button">立即更新</button>
      </footer>
    </section>
  </div>
</template>
