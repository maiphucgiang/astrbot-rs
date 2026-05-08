<template>
  <div class="plugins-page">
    <div class="header-row">
      <h2>插件管理</h2>
      <button class="btn" @click="showAdd = true" v-if="!showAdd">+ 安装插件</button>
    </div>

    <div v-if="showAdd" class="add-form card">
      <h3>安装插件</h3>
      <div class="form-row">
        <label>Git 地址</label>
        <input v-model="newUrl" placeholder="https://github.com/xxx/xxx.git" />
      </div>
      <div class="form-actions">
        <button class="btn" @click="install" :disabled="installing || !newUrl.trim()">
          {{ installing ? '安装中...' : '安装' }}
        </button>
        <button class="btn btn-outline" @click="showAdd = false">取消</button>
      </div>
      <p v-if="msg" :class="['hint', msgOk ? 'ok' : 'err']">{{ msg }}</p>
    </div>

    <table class="table">
      <thead><tr><th>插件</th><th>版本</th><th>状态</th><th>操作</th></tr></thead>
      <tbody>
        <tr v-for="p in plugins" :key="p.id">
          <td>
            <strong>{{ p.name }}</strong><br/>
            <small style="color:var(--text-muted)">{{ p.description }}</small>
          </td>
          <td>{{ p.version }}</td>
          <td>
            <span :class="['tag', p.enabled ? 'on' : 'off']">{{ p.enabled ? '启用' : '禁用' }}</span>
          </td>
          <td>
            <button class="btn btn-sm btn-outline" @click="toggle(p)">
              {{ p.enabled ? '禁用' : '启用' }}
            </button>
          </td>
        </tr>
        <tr v-if="!plugins.length">
          <td colspan="4" class="empty">暂无插件</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'

const plugins = ref([])
const showAdd = ref(false)
const newUrl = ref('')
const installing = ref(false)
const msg = ref('')
const msgOk = ref(false)

async function load() {
  try {
    const r = await fetch('/api/plugins')
    const d = await r.json()
    plugins.value = d.plugins || []
  } catch {
    plugins.value = []
  }
}

async function install() {
  const url = newUrl.value.trim()
  if (!url) return
  installing.value = true
  msg.value = ''
  try {
    const r = await fetch('/api/plugins', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ url })
    })
    const d = await r.json()
    if (r.ok) {
      msg.value = '安装成功'
      msgOk.value = true
      newUrl.value = ''
      showAdd.value = false
      await load()
    } else {
      msg.value = d.error || '安装失败'
      msgOk.value = false
    }
  } catch (e) {
    msg.value = '网络错误: ' + e.message
    msgOk.value = false
  }
  installing.value = false
}

async function toggle(p) {
  try {
    const r = await fetch(`/api/plugins/${p.id}`, {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ enabled: !p.enabled })
    })
    if (r.ok) await load()
  } catch {}
}

onMounted(load)
</script>

<style scoped>
.plugins-page { padding: 24px; }
.header-row { display:flex; align-items:center; justify-content:space-between; margin-bottom:20px; }
.plugins-page h2 { font-size: 20px; color: #e8e8e8; }
.add-form { max-width: 480px; margin-bottom: 20px; }
.add-form h3 { font-size: 16px; margin-bottom: 12px; color: #e8e8e8; }
.form-row { margin-bottom: 12px; }
.form-row label { display:block; font-size:13px; color:#9ca3af; margin-bottom:6px; }
.form-row input {
  width:100%; padding:10px 12px; border:1px solid #2a2a3c; border-radius:8px;
  background:#0d0d12; color:#e8e8e8; font-size:14px; outline:none;
}
.form-row input:focus { border-color:#a855f7; }
.form-actions { display:flex; gap:10px; }
.hint { margin-top:10px; font-size:13px; }
.hint.ok { color:#22c55e; }
.hint.err { color:#ef4444; }
</style>
