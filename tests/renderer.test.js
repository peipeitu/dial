const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const root = path.resolve(__dirname, "..");
const html = fs.readFileSync(path.join(root, "src", "renderer", "index.html"), "utf8");
const rendererSource = fs
  .readFileSync(path.join(root, "src", "renderer", "renderer.js"), "utf8")
  .replace(/\nboot\(\);\s*$/, "\n");

function createElement() {
  const attributes = new Map();
  const styleValues = new Map();
  return {
    append() {},
    appendChild() {},
    addEventListener() {},
    classList: {
      add() {},
      remove() {},
      toggle() {}
    },
    closest() {
      return this;
    },
    dataset: {},
    disabled: false,
    focus() {},
    getBoundingClientRect() {
      return { bottom: 0, height: 0, left: 0, top: 0, width: 720 };
    },
    hidden: true,
    removeAttribute(name) {
      attributes.delete(name);
    },
    replaceChildren() {},
    scrollIntoView() {},
    setAttribute(name, value) {
      attributes.set(name, String(value));
    },
    getAttribute(name) {
      return attributes.get(name) ?? null;
    },
    style: {
      removeProperty(name) {
        styleValues.delete(name);
      },
      setProperty(name, value) {
        styleValues.set(name, String(value));
      },
      getPropertyValue(name) {
        return styleValues.get(name) ?? "";
      }
    },
    textContent: "",
    value: ""
  };
}

function loadRenderer() {
  const elements = new Map();
  const document = {
    body: createElement(),
    createElement,
    documentElement: { lang: "" },
    addEventListener() {},
    getElementById(id) {
      if (!elements.has(id)) elements.set(id, createElement());
      return elements.get(id);
    },
    querySelector(selector) {
      return selector === "title" ? this.getElementById("documentTitle") : null;
    },
    querySelectorAll() {
      return [];
    }
  };
  const mediaQuery = {
    addEventListener() {},
    matches: false
  };
  const context = vm.createContext({
    document,
    navigator: {
      language: "zh-CN",
      languages: ["zh-CN"],
      platform: "MacIntel"
    },
    window: {
      aiUsage: {
        platform: "darwin"
      },
      addEventListener() {},
      clearInterval() {},
      clearTimeout() {},
      matchMedia() {
        return mediaQuery;
      },
      open() {},
      setInterval() {
        return 1;
      },
      setTimeout() {
        return 1;
      }
    }
  });

  vm.runInContext(rendererSource, context, { filename: "renderer.js" });
  return { context, elements };
}

test("every renderer element id exists in the HTML shell", () => {
  const htmlIds = new Set(Array.from(html.matchAll(/\bid="([^"]+)"/g), (match) => match[1]));
  const rendererIds = new Set(
    Array.from(rendererSource.matchAll(/document\.getElementById\("([^"]+)"\)/g), (match) => match[1])
  );
  const missing = Array.from(rendererIds).filter((id) => !htmlIds.has(id));

  assert.deepEqual(missing, []);
});

test("today token metric uses today's total and period share", () => {
  const { context, elements } = loadRenderer();

  vm.runInContext(
    `
      elements.todayTokens.textContent = formatCompact(1500);
      updateTodayTokensMeter(statsRatioPercent(1500, 10000));
    `,
    context
  );

  assert.equal(elements.get("todayTokens").textContent, "1.5K");
  assert.equal(elements.get("todayTokensMeter").style.getPropertyValue("--today-token-progress"), "15%");
  assert.equal(elements.get("todayTokensMeter").getAttribute("aria-valuenow"), "15");
});

test("darwin uses macOS provider defaults instead of Windows paths", () => {
  const { context } = loadRenderer();

  const defaults = vm.runInContext(
    `({
      copilot: defaultCopilotHome(),
      cursor: defaultCursorHome(),
      chatgpt: defaultChatgptHome()
    })`,
    context
  );

  assert.equal(defaults.copilot, "~/Library/Application Support/Code/User/globalStorage/github.copilot-chat");
  assert.equal(defaults.cursor, "~/Library/Application Support/Cursor/User/globalStorage");
  assert.equal(defaults.chatgpt, "~/Library/Application Support/com.openai.chat");
});

test("unpriced providers show an explicit unavailable cost state", () => {
  const { context, elements } = loadRenderer();

  vm.runInContext(
    `
      currentSettings.language = "zh";
      renderCostMetricLabels(30);
      renderCostMetricValues({
        featured: {
          costAvailable: false,
          todayCost: 0,
          periodCost: 0
        }
      });
    `,
    context
  );

  assert.equal(elements.get("todayCost").textContent, "不可用");
  assert.equal(elements.get("periodCost").textContent, "不可用");
  assert.equal(elements.get("todayCost").getAttribute("title"), "暂无可靠定价");
});

test("scan diagnostics render the active provider cache result", () => {
  const { context, elements } = loadRenderer();

  vm.runInContext(
    `
      currentSettings.language = "zh";
      currentSettings.activeProvider = "codex";
      scanDiagnostics = [{
        provider: "codex",
        elapsedMs: 42,
        totalFiles: 10,
        cacheHits: 8,
        parsedFiles: 2,
        deletedFiles: 1,
        failedFiles: 0,
        cacheHitRate: 80
      }];
      renderScanDiagnostics();
    `,
    context
  );

  assert.equal(elements.get("scanProviderValue").textContent, "Codex");
  assert.equal(elements.get("scanDiagnosticsSummary").textContent, "42 ms · 命中 8/10（80%）");
  assert.equal(elements.get("scanDiagnosticsMeta").textContent, "重新解析 2 · 删除 1 · 失败 0");
  assert.equal(elements.get("rebuildCacheStatus").textContent, "缓存运行正常");
});
