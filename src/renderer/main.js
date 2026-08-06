import { createApp, nextTick } from "vue";
import App from "./App.vue";
import "./styles.css";

window.__AI_USAGE_ASSETS__ = {
  app: new URL("./assets/app-icon.svg", import.meta.url).href,
  appDark: new URL("./assets/app-icon-dark.svg", import.meta.url).href,
  codex: new URL("./assets/provider-codex-light.svg", import.meta.url).href,
  codexLight: new URL("./assets/provider-codex-light.svg", import.meta.url).href,
  codexDark: new URL("./assets/provider-codex-dark.svg", import.meta.url).href,
  claude: new URL("./assets/provider-claude.svg", import.meta.url).href,
  copilot: new URL("./assets/provider-copilot.svg", import.meta.url).href,
  cursor: new URL("./assets/provider-cursor.svg", import.meta.url).href,
  chatgpt: new URL("./assets/provider-chatgpt.svg", import.meta.url).href
};

async function bootstrap() {
  createApp(App).mount("#app");
  await nextTick();
  await import("./renderer.js");
}

bootstrap();
