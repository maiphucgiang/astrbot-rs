<template>
  <div class="chat">
    <div class="messages" ref="scroll">
      <div v-for="m in messages" :key="m.id" :class="['msg', m.role]">
        <div class="bubble">{{ m.content }}</div>
        <div class="meta">{{ m.time }}</div>
      </div>
    </div>
    <div class="input-bar">
      <input v-model="text" @keydown.enter="send" placeholder="输入消息..." />
      <button @click="send" :disabled="!ws || ws.readyState !== 1">发送</button>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, onUnmounted, inject, nextTick } from 'vue'

const messages = ref([{ id: 'welcome', role: 'assistant', content: '你好，我是 AstrBot。有什么可以帮你的？', time: now() }])
const text = ref('')
const scroll = ref(null)
const wsConnected = inject('wsConnected')
let ws = null
let sid = localStorage.getItem('ab_session') || ''

function now() { return new Date().toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' }) }

function append(role, content) {
  messages.value.push({ id: crypto.randomUUID(), role, content, time: now() })
  nextTick(() => { scroll.value.scrollTop = scroll.value.scrollHeight })
}

function connect() {
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:'
  ws = new WebSocket(`${proto}//${location.host}/ws/chat`)
  ws.onopen = () => { wsConnected.value = true }
  ws.onmessage = (ev) => {
    try {
      const d = JSON.parse(ev.data)
      if (d.error && !d.reply) { append('system', `错误: ${d.error}`); return }
      if (d.reply) { append('assistant', d.reply) }
      if (d.session_id) { sid = d.session_id; localStorage.setItem('ab_session', sid) }
    } catch { append('assistant', ev.data) }
  }
  ws.onclose = () => { wsConnected.value = false; setTimeout(connect, 3000) }
}

function send() {
  const t = text.value.trim()
  if (!t || !ws || ws.readyState !== 1) return
  append('user', t)
  ws.send(JSON.stringify({ message: t, session_id: sid }))
  text.value = ''
}

onMounted(connect)
onUnmounted(() => ws && ws.close())
</script>

<style scoped>
.chat { height: 100%; display: flex; flex-direction: column; }
.messages { flex: 1; overflow-y: auto; padding: 16px; display: flex; flex-direction: column; gap: 12px; }
.msg { display: flex; flex-direction: column; max-width: 70%; }
.msg.user { align-self: flex-end; }
.msg.assistant { align-self: flex-start; }
.msg.system { align-self: center; max-width: 90%; }
.bubble {
  padding: 10px 14px;
  border-radius: 12px;
  font-size: 14px;
  line-height: 1.5;
  word-break: break-word;
}
.msg.user .bubble { background: #7c3aed; color: #fff; border-bottom-right-radius: 4px; }
.msg.assistant .bubble { background: #1f1f2e; color: #e8e8e8; border: 1px solid #2a2a3c; border-bottom-left-radius: 4px; }
.msg.system .bubble { background: #1a1a24; color: #9ca3af; font-size: 12px; }
.meta { font-size: 11px; color: #6b7280; margin-top: 4px; }
.msg.user .meta { text-align: right; }
.input-bar {
  padding: 12px 16px;
  background: #13131a;
  border-top: 1px solid #1a1a24;
  display: flex;
  gap: 10px;
}
.input-bar input {
  flex: 1;
  padding: 10px 14px;
  border: 1px solid #2a2a3c;
  border-radius: 8px;
  background: #0d0d12;
  color: #e8e8e8;
  font-size: 14px;
  outline: none;
}
.input-bar input:focus { border-color: #a855f7; }
.input-bar button {
  padding: 10px 18px;
  border: none;
  border-radius: 8px;
  background: #7c3aed;
  color: #fff;
  font-size: 14px;
  cursor: pointer;
}
.input-bar button:disabled { background: #4b5563; cursor: not-allowed; }
</style>
