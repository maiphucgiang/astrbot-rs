<template>
  <div class="app">
    <aside class="sidebar">
      <div class="brand">AstrBot</div>
      <nav>
        <button v-for="item in nav" :key="item.key" :class="{ active: page === item.key }" @click="page = item.key">
          {{ item.label }}
        </button>
      </nav>
      <div class="status">
        <span class="dot" :class="{ connected: wsConnected }"></span>
        {{ wsConnected ? '已连接' : '未连接' }}
      </div>
    </aside>
    <main class="main">
      <Chat v-if="page === 'chat'" />
      <Status v-else-if="page === 'status'" />
      <Providers v-else-if="page === 'providers'" />
    </main>
  </div>
</template>

<script setup>
import { ref, provide } from 'vue'
import Chat from './Chat.vue'
import Status from './Status.vue'
import Providers from './Providers.vue'

const page = ref('chat')
const wsConnected = ref(false)

provide('wsConnected', wsConnected)

const nav = [
  { key: 'chat', label: '对话' },
  { key: 'status', label: '状态' },
  { key: 'providers', label: 'Providers' },
]
</script>

<style>
* { margin: 0; padding: 0; box-sizing: border-box; }
html, body, #app, .app { height: 100%; }
body {
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  background: #0d0d12;
  color: #e8e8e8;
}
.app { display: flex; }
.sidebar {
  width: 200px;
  background: #13131a;
  border-right: 1px solid #1a1a24;
  display: flex;
  flex-direction: column;
  padding: 16px 0;
}
.brand {
  padding: 0 16px 20px;
  font-size: 18px;
  font-weight: 700;
  color: #a855f7;
  border-bottom: 1px solid #1a1a24;
  margin-bottom: 8px;
}
nav button {
  display: block;
  width: 100%;
  text-align: left;
  padding: 10px 16px;
  background: none;
  border: none;
  color: #9ca3af;
  font-size: 14px;
  cursor: pointer;
  transition: .15s;
}
nav button:hover { color: #e8e8e8; background: #1a1a24; }
nav button.active { color: #a855f7; background: #1a1a24; border-right: 2px solid #a855f7; }
.status {
  margin-top: auto;
  padding: 12px 16px;
  font-size: 12px;
  color: #6b7280;
  display: flex;
  align-items: center;
  gap: 6px;
  border-top: 1px solid #1a1a24;
}
.dot { width: 7px; height: 7px; border-radius: 50%; background: #ef4444; }
.dot.connected { background: #22c55e; }
.main { flex: 1; overflow: hidden; }
</style>
