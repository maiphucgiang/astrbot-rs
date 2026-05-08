<template>
  <div class="settings-page">
    <h2>系统设置</h2>
    <div class="card" style="max-width:600px">
      <div v-for="(value,key) in settings" :key="key" class="setting-row">
        <div class="setting-info">
          <span class="setting-key">{{ key }}</span>
          <span class="setting-value">{{ value }}</span>
        </div>
        <button class="btn btn-sm btn-outline" @click="edit(key)">编辑</button>
      </div>
    </div>

    <div v-if="editingKey" class="card edit-card" style="max-width:480px">
      <h3>编辑 {{ editingKey }}</h3>
      <input v-model="editValue" />
      <div class="form-actions">
        <button class="btn" @click="save" :disabled="saving">{{ saving ? '保存中...' : '保存' }}</button>
        <button class="btn btn-outline" @click="editingKey = null">取消</button>
      </div>
      <p v-if="msg" :class="['hint', msgOk ? 'ok' : 'err']">{{ msg }}</p>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'

const settings = ref({})
const editingKey = ref(null)
const editValue = ref('')
const saving = ref(false)
const msg = ref('')
const msgOk = ref(false)

async function load() {
  try {
    const r = await fetch('/api/config')
    const d = await r.json()
    settings.value = d.config || {
      'Dashboard Port': '6185',
      'Log Level': 'info',
      'Timezone': 'Asia/Shanghai',
      'Default Persona': 'default',
    }
  } catch {
    settings.value = {
      'Dashboard Port': '6185',
      'Log Level': 'info',
      'Timezone': 'Asia/Shanghai',
    }
  }
}

function edit(key) {
  editingKey.value = key
  editValue.value = String(settings.value[key])
}

async function save() {
  saving.value = true
  msg.value = ''
  try {
    const r = await fetch('/api/config', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ [editingKey.value]: editValue.value })
    })
    if (r.ok) {
      msg.value = '保存成功'
      msgOk.value = true
      settings.value[editingKey.value] = editValue.value
      editingKey.value = null
    } else {
      const d = await r.json().catch(() => ({}))
      msg.value = d.error || '保存失败'
      msgOk.value = false
    }
  } catch (e) {
    msg.value = '网络错误: ' + e.message
    msgOk.value = false
  }
  saving.value = false
}

onMounted(load)
</script>

<style scoped>
.settings-page { padding: 24px; }
.settings-page h2 { font-size: 20px; margin-bottom: 20px; color: #e8e8e8; }
.setting-row {
  display: flex; justify-content: space-between; align-items: center;
  padding: 14px 0; border-bottom: 1px solid #1a1a24;
}
.setting-key { font-size: 14px; color: #e8e8e8; }
.setting-value { font-size: 14px; color: #6b7280; margin-left: 12px; }
.edit-card { margin-top: 16px; }
.edit-card h3 { font-size: 16px; margin-bottom: 12px; color: #e8e8e8; }
.edit-card input {
  width:100%; padding:10px 12px; border:1px solid #2a2a3c; border-radius:8px;
  background:#0d0d12; color:#e8e8e8; font-size:14px; outline:none; margin-bottom:12px;
}
.form-actions { display:flex; gap:10px; }
.hint { margin-top:10px; font-size:13px; }
.hint.ok { color:#22c55e; }
.hint.err { color:#ef4444; }
</style>
