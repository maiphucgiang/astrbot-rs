<template>
  <div class="providers-page">
    <div class="header-row">
      <h2>Providers</h2>
      <button class="btn" @click="showAdd = true" v-if="!showAdd">+ 添加 Provider</button>
    </div>

    <div v-if="showAdd" class="add-form card">
      <h3>添加 Provider</h3>
      <div class="form-row"><label>ID</label><input v-model="newForm.id" placeholder="my-openai" /></div>
      <div class="form-row"><label>类型</label>
        <select v-model="newForm.provider_type">
          <option value="openai">OpenAI</option>
          <option value="openai_compatible">OpenAI Compatible</option>
          <option value="anthropic">Anthropic</option>
        </select>
      </div>
      <div class="form-row"><label>API Key</label><input v-model="newForm.api_key" placeholder="sk-..." /></div>
      <div class="form-row"><label>Base URL</label><input v-model="newForm.base_url" placeholder="https://api.openai.com" /></div>
      <div class="form-row"><label>模型</label><input v-model="newForm.model" placeholder="gpt-4o-mini" /></div>
      <div class="form-actions">
        <button class="btn" @click="add" :disabled="adding || !newForm.id.trim()">{{ adding ? '添加中...' : '添加' }}</button>
        <button class="btn btn-outline" @click="showAdd = false">取消</button>
      </div>
      <p v-if="msg" :class="['hint', msgOk ? 'ok' : 'err']">{{ msg }}</p>
    </div>

    <table class="table">
      <thead><tr><th>ID</th><th>类型</th><th>模型</th><th>状态</th><th>操作</th></tr></thead>
      <tbody>
        <tr v-for="p in providers" :key="p.id">
          <td><strong>{{ p.id }}</strong></td>
          <td>{{ p.provider_type }}</td>
          <td>{{ p.model }}</td>
          <td><span :class="['tag', p.enabled ? 'on' : 'off']">{{ p.enabled ? '启用' : '禁用' }}</span></td>
          <td><button class="btn btn-sm btn-outline" @click="test(p)">测试</button></td>
        </tr>
        <tr v-if="!providers.length"><td colspan="5" class="empty">暂无 Providers</td></tr>
      </tbody>
    </table>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'

const providers = ref([])
const showAdd = ref(false)
const newForm = ref({ id: '', provider_type: 'openai', api_key: '', base_url: '', model: '' })
const adding = ref(false)
const msg = ref('')
const msgOk = ref(false)

async function load() {
  try {
    const r = await fetch('/api/providers')
    const d = await r.json()
    providers.value = d.providers || []
  } catch {
    providers.value = []
  }
}

async function add() {
  adding.value = true
  msg.value = ''
  try {
    const r = await fetch('/api/providers', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(newForm.value)
    })
    const d = await r.json()
    if (r.ok) {
      msg.value = '添加成功'
      msgOk.value = true
      newForm.value = { id: '', provider_type: 'openai', api_key: '', base_url: '', model: '' }
      showAdd.value = false
      await load()
    } else {
      msg.value = d.error || '添加失败'
      msgOk.value = false
    }
  } catch (e) {
    msg.value = '网络错误: ' + e.message
    msgOk.value = false
  }
  adding.value = false
}

async function test(p) {
  try {
    const r = await fetch(`/api/providers/${p.id}/test`, { method: 'POST' })
    const d = await r.json()
    alert(d.ok ? '连接成功' : ('连接失败: ' + (d.error || '未知错误')))
  } catch (e) {
    alert('测试失败: ' + e.message)
  }
}

onMounted(load)
</script>

<style scoped>
.providers-page { padding: 24px; }
.header-row { display:flex; align-items:center; justify-content:space-between; margin-bottom:20px; }
.providers-page h2 { font-size: 20px; color: #e8e8e8; }
.add-form { max-width: 480px; margin-bottom: 20px; }
.add-form h3 { font-size: 16px; margin-bottom: 12px; color: #e8e8e8; }
.form-row { margin-bottom: 12px; }
.form-row label { display:block; font-size:13px; color:#9ca3af; margin-bottom:6px; }
.form-row input, .form-row select {
  width:100%; padding:10px 12px; border:1px solid #2a2a3c; border-radius:8px;
  background:#0d0d12; color:#e8e8e8; font-size:14px; outline:none;
}
.form-row input:focus, .form-row select:focus { border-color:#a855f7; }
.form-actions { display:flex; gap:10px; }
.hint { margin-top:10px; font-size:13px; }
.hint.ok { color:#22c55e; }
.hint.err { color:#ef4444; }
</style>
