<template>
  <div class="providers-page">
    <h2>Providers</h2>
    <table>
      <thead>
        <tr>
          <th>ID</th>
          <th>类型</th>
          <th>模型</th>
          <th>状态</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="p in providers" :key="p.id">
          <td>{{ p.id }}</td>
          <td>{{ p.provider_type }}</td>
          <td>{{ p.model }}</td>
          <td>
            <span :class="['tag', p.enabled ? 'on' : 'off']">{{ p.enabled ? '启用' : '禁用' }}</span>
          </td>
        </tr>
      </tbody>
    </table>
    <p v-if="!providers.length" class="empty">暂无 Providers</p>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'

const providers = ref([])

async function load() {
  try {
    const r = await fetch('/api/providers')
    const d = await r.json()
    providers.value = d.providers || []
  } catch {
    providers.value = []
  }
}

onMounted(load)
</script>

<style scoped>
.providers-page { padding: 24px; }
.providers-page h2 { font-size: 20px; margin-bottom: 20px; color: #e8e8e8; }
table { width: 100%; border-collapse: collapse; font-size: 14px; }
th, td { padding: 10px 12px; text-align: left; border-bottom: 1px solid #1a1a24; }
th { color: #9ca3af; font-weight: 500; }
td { color: #e8e8e8; }
.tag { display: inline-block; padding: 2px 8px; border-radius: 4px; font-size: 12px; }
.tag.on { background: #064e3b; color: #34d399; }
.tag.off { background: #450a0a; color: #f87171; }
.empty { color: #6b7280; margin-top: 16px; }
</style>
