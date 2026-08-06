<script setup>
import AppHeader from "./components/AppHeader.vue";
import SettingsView from "./components/SettingsView.vue";
import UpdateDialog from "./components/UpdateDialog.vue";
import UsageBrief from "./components/UsageBrief.vue";
</script>

<template>
  <main class="app-shell">
    <AppHeader />
    <section class="workspace">
      <section id="errorPanel" class="error-panel" hidden></section>
      <UsageBrief />
      <SettingsView />
    </section>
  </main>
  <UpdateDialog />
</template>
