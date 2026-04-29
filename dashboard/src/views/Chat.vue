<template>
  <div class="chat-container">
    <PageHeader title="WebChat" />
    <div class="messages" ref="messagesRef">
      <div
        v-for="msg in messages"
        :key="msg.id"
        :class="['message-bubble', msg.role === 'user' ? 'user' : 'assistant']"
      >
        <div class="meta">{{ msg.role === 'user' ? '我' : 'AstrBot' }} · {{ formatTime(msg.created_at) }}</div>
        <div class="content">{{ msg.content }}</div>
      </div>
      <div v-if="connecting" class="status">连接中...</div>
      <div v-else-if="!connected" class="status error">连接断开</div>
      <div v-if="sending" class="status">发送中...</div>
    </div>
    <div class="input-bar">
      <el-input
        v-model="inputText"
        placeholder="输入消息..."
        :disabled="!connected || sending"
        @keydown.enter="send"
        clearable
      />
      <el-button type="primary" :disabled="!connected || sending || !inputText.trim()" @click="send">
        发送
      </el-button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted, nextTick } from 'vue'
import PageHeader from '@/components/PageHeader.vue'

interface ChatMessage {
  id: string
  role: 'user' | 'assistant'
  content: string
  created_at?: string
}

const messages = ref<ChatMessage[]>([])
const inputText = ref('')
const connected = ref(false)
const connecting = ref(true)
const sending = ref(false)
const messagesRef = ref<HTMLDivElement>()
const sessionId = ref('')

let ws: WebSocket | null = null

function connect() {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  const url = `${protocol}//${window.location.host}/ws/chat`
  ws = new WebSocket(url)

  ws.onopen = () => {
    connected.value = true
    connecting.value = false
  }

  ws.onmessage = (event) => {
    try {
      const data = JSON.parse(event.data)
      if (data.error) {
        messages.value.push({
          id: 'err_' + Date.now(),
          role: 'assistant',
          content: '[错误] ' + data.error,
          created_at: new Date().toISOString()
        })
        sending.value = false
        scrollToBottom()
        return
      }
      if (data.reply) {
        messages.value.push({
          id: 'bot_' + Date.now(),
          role: 'assistant',
          content: data.reply,
          created_at: new Date().toISOString()
        })
        if (data.session_id) {
          sessionId.value = data.session_id
        }
        sending.value = false
        scrollToBottom()
      }
    } catch (e) {
      console.error('WS parse error:', e)
    }
  }

  ws.onclose = () => {
    connected.value = false
    connecting.value = false
    // auto reconnect after 3s
    setTimeout(() => connect(), 3000)
  }

  ws.onerror = (err) => {
    console.error('WS error:', err)
    connected.value = false
    connecting.value = false
  }
}

function send() {
  const text = inputText.value.trim()
  if (!text || !ws || !connected.value || sending.value) return

  sending.value = true
  messages.value.push({
    id: 'user_' + Date.now(),
    role: 'user',
    content: text,
    created_at: new Date().toISOString()
  })
  inputText.value = ''
  scrollToBottom()

  ws.send(JSON.stringify({
    message: text,
    session_id: sessionId.value || undefined
  }))
}

function scrollToBottom() {
  nextTick(() => {
    if (messagesRef.value) {
      messagesRef.value.scrollTop = messagesRef.value.scrollHeight
    }
  })
}

function formatTime(iso?: string) {
  if (!iso) return ''
  try {
    const d = new Date(iso)
    return d.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
  } catch {
    return ''
  }
}

onMounted(() => {
  connect()
})

onUnmounted(() => {
  if (ws) {
    ws.close()
    ws = null
  }
})
</script>

<style scoped>
.chat-container {
  display: flex;
  flex-direction: column;
  height: calc(100vh - 120px);
  max-width: 900px;
  margin: 0 auto;
}
.messages {
  flex: 1;
  overflow-y: auto;
  padding: 16px;
  background: #fff;
  border-radius: 8px;
  margin-bottom: 12px;
}
.message-bubble {
  margin-bottom: 12px;
  max-width: 80%;
}
.message-bubble.user {
  margin-left: auto;
  text-align: right;
}
.message-bubble.assistant {
  margin-right: auto;
  text-align: left;
}
.meta {
  font-size: 12px;
  color: #888;
  margin-bottom: 4px;
}
.content {
  display: inline-block;
  padding: 10px 14px;
  border-radius: 12px;
  font-size: 14px;
  line-height: 1.5;
  word-break: break-word;
}
.message-bubble.user .content {
  background: #409eff;
  color: #fff;
}
.message-bubble.assistant .content {
  background: #f2f3f5;
  color: #333;
}
.status {
  text-align: center;
  color: #888;
  font-size: 12px;
  padding: 8px;
}
.status.error {
  color: #f56c6c;
}
.input-bar {
  display: flex;
  gap: 8px;
  padding: 0 4px;
}
.input-bar .el-input {
  flex: 1;
}
</style>
