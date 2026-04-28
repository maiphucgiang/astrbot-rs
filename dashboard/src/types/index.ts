export interface Stats {
  healthy: boolean
  total_messages: number
  active_adapters: number
  active_providers: number
}

export interface Provider {
  id: string
  name: string
  type: 'llm' | 'tts' | 'stt' | 'embedding'
  enabled: boolean
  config: Record<string, any>
}

export interface Adapter {
  id: string
  name: string
  connection_mode: string
  enabled: boolean
}

export interface Message {
  id: string
  timestamp: string
  platform: string
  sender: string
  content: string
}
