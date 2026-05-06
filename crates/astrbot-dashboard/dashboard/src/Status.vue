<template>
  <div class="status-page">
    <h2>系统状态</h2>
    <div class="cards">
      <div class="card" v-for="c in cards" :key="c.title">
        <div class="card-title">{{ c.title }}</div>
        <div class="card-value" :class="c.class">{{ c.value }}</div>
      </div>
    </div>
    <div class="actions">
      <button @click="refresh">刷新</button>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'

const cards = ref([
  { title: '运行状态', value: '运行中', class: 'ok' },
  { title: 'Providers', value: '-', class: '' },
  { title: 'Plugins', value: '-', class: '' },
  { title: 'Platforms', value: '-', class: '' },
])

async function refresh() {
  try {
    const r = await fetch('/api/status')
    const d = await r.json()
    cards.value[0].value = d.status || '运行中'
    cards.value[1].value = String(d.metrics?.providers_count ?? '-')
    cards.value[2].value = String(d.metrics?.plugins_count ?? '-')
    cards.value[3].value = String(d.metrics?.platforms_count ?? '-')
  } catch {
    cards.value[0].value = 'API 错误'
  }
}

onMounted(refresh)
</script>

<style scoped>
.status-page { padding: 24px; }
.status-page h2 { font-size: 20px; margin-bottom: 20px; color: #e8e8e8; }
.cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: 16px; }
.card {
  background: #13131a;
  border: 1px solid #1a1a24;
  border-radius: 10px;
  padding: 16px;
}
.card-title { font-size: 12px; color: #9ca3af; margin-bottom: 8px; }
.card-value { font-size: 24px; font-weight: 700; color: #e8e8e8; }
.card-value.ok { color: #22c55e; }
.actions { margin-top: 20px; }
.actions button {
  padding: 8px 16px;
  border: 1px solid #2a2a3c;
  border-radius: 6px;
  background: #1a1a24;
  color: #e8e8e8;
  cursor: pointer;
}

/* Shared utility classes */
.btn { padding: 8px 16px; border-radius: 8px; border: none; background: #7c3aed; color: #fff; font-size: 13px; cursor: pointer; }
.btn:hover { background: #a78bfa; }
.btn-outline { background: transparent; border: 1px solid #2a2a3c; color: #9ca3af; }
.btn-outline:hover { border-color: #a855f7; color: #a855f7; }
.btn-sm { padding: 6px 12px; font-size: 12px; }
.table { width: 100%; border-collapse: collapse; font-size: 14px; }
.table th, .table td { padding: 10px 12px; text-align: left; border-bottom: 1px solid #1a1a24; }
.table th { color: #9ca3af; font-weight: 500; }
.tag { display: inline-block; padding: 2px 8px; border-radius: 4px; font-size: 12px; }
.tag.on { background: #064e3b; color: #34d399; }
.tag.off { background: #450a0a; color: #f87171; }
.empty { color: #6b7280; text-align: center; padding: 16px 0; }
</style>
