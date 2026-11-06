const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

function createElement() {
  return {
    children: [],
    attributes: {},
    append(...children) { this.children.push(...children); },
    replaceChildren(...children) { this.children = children; },
    handlers: {},
    addEventListener(name, handler) { this.handlers[name] = handler; },
    disabled: false,
    setAttribute(name, value) { this.attributes[name] = value; },
    style: {},
    textContent: ""
  };
}

function loadTrayScript(tauri = {}) {
  const elements = new Map();
  const document = {
    body: { dataset: {} },
    documentElement: { lang: "" },
    createElement,
    getElementById(id) {
      if (!elements.has(id)) elements.set(id, createElement());
      return elements.get(id);
    }
  };
  const context = vm.createContext({
    document,
    window: {
      __TAURI__: tauri,
      setInterval() {},
      addEventListener() {}
    }
  });
  const source = fs.readFileSync(path.join(__dirname, "..", "src", "renderer", "tray.js"), "utf8");

  vm.runInContext(source, context, { filename: "tray.js" });
  return { context, elements };
}

test("missing remaining usage stays unavailable", () => {
  const { context, elements } = loadTrayScript();

  assert.equal(context.normalizedPercent(null), null);
  assert.equal(context.normalizedPercent(undefined), null);
  assert.equal(context.normalizedPercent(0), 0);

  context.renderStatus({
    provider: "copilot",
    remainingPercent: null,
    statusLabel: "Usage unavailable"
  });

  assert.equal(elements.get("usageValue").textContent, "--");
  assert.equal(elements.get("usageFill").style.width, "0");
  assert.equal(elements.get("usageMeta").textContent, "Usage unavailable");
});

test("quota rows show each period, segmented remaining usage and live reset countdowns", () => {
  const { context, elements } = loadTrayScript();
  const weeklyReset = new Date(Date.now() + (6 * 24 + 6) * 3600000).toISOString();
  context.renderStatus({
    language: "zh-CN",
    windows: [
      { windowMinutes: 10080, remainingPercent: 78, resetsAt: weeklyReset },
      { windowMinutes: 300, remainingPercent: 0, resetsAt: null }
    ]
  });
  const [short, weekly] = elements.get("quotaList").children;
  assert.equal(elements.get("usageSummary").hidden, true);
  assert.equal(short.children[0].children[0].textContent, "5 小时");
  assert.equal(short.children[1].children.length, 5);
  assert.equal(short.children[1].attributes["aria-valuenow"], "0");
  assert.equal(short.children[2].textContent, "更新时间未知");
  assert.equal(weekly.children[0].children[0].textContent, "每周");
  assert.equal(weekly.children[0].children[1].textContent, "78% 剩余");
  assert.equal(weekly.children[1].children.length, 7);
  assert.deepEqual(weekly.children[1].children.map(segment => segment.children[0].style.width),
    ["100%", "100%", "100%", "100%", "100%", "46%", "0%"]);
  assert.equal(weekly.children[2].textContent, "更新 6天 6小时");
  weekly.children[2].textContent = "stale";
  context.updateCountdowns();
  assert.equal(weekly.children[2].textContent, "更新 6天 6小时");

  context.renderStatus({ provider: "claude", windows: [] });
  assert.equal(elements.get("quotaList").children.length, 0);
  assert.equal(elements.get("usageSummary").hidden, false);
});

test("reset countdown handles boundaries, missing dates and English", () => {
  const { context } = loadTrayScript();
  const now = Date.parse("2026-11-07T00:00:00Z");
  const at = seconds => new Date(now + seconds * 1000).toISOString();
  assert.equal(context.resetCountdown(at(0), now), "即将更新");
  assert.equal(context.resetCountdown(at(-60), now), "即将更新");
  assert.equal(context.resetCountdown(at(61), now), "更新 1分 1秒");
  assert.equal(context.resetCountdown(at(3660), now), "更新 1小时 1分");
  assert.equal(context.resetCountdown("invalid", now), "更新时间未知");
  context.renderStatus({ language: "en" });
  assert.equal(context.quotaLabel(10080), "Weekly");
  assert.equal(context.resetCountdown(at(90000), now), "Updates in 1d 1h");
});
