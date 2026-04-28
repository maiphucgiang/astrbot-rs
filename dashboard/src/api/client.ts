import axios from 'axios'

export const api = axios.create({
  baseURL: import.meta.env.VITE_API_BASE_URL || '',
  timeout: 30000,
  headers: {
    'Content-Type': 'application/json'
  }
})

api.interceptors.response.use(
  (response) => response,
  (error) => {
    console.error('API Error:', error)
    return Promise.reject(error)
  }
)

// ===== 系统状态 =====
export const getHealth = () => api.get('/api/health')
export const getStatus = () => api.get('/api/status')
export const getDetailedStatus = () => api.get('/api/status/detailed')

// ===== 配置 =====
export const getConfig = () => api.get('/api/config')
export const updateConfig = (body: any) => api.put('/api/config', body)
export const getConfigKey = (key: string) => api.get(`/api/config/${key}`)
export const updateConfigKey = (key: string, body: any) => api.put(`/api/config/${key}`, body)

// ===== 提供商 =====
export const listProviders = () => api.get('/api/providers')
export const getProvider = (id: string) => api.get(`/api/providers/${id}`)
export const updateProvider = (id: string, body: any) => api.put(`/api/providers/${id}`, body)
export const deleteProvider = (id: string) => api.delete(`/api/providers/${id}`)
export const testProvider = (id: string) => api.post(`/api/providers/${id}/test`)

// ===== 平台适配器 =====
export const listPlatforms = () => api.get('/api/platforms')
export const getPlatform = (id: string) => api.get(`/api/platforms/${id}`)
export const updatePlatform = (id: string, body: any) => api.put(`/api/platforms/${id}`, body)

// ===== 插件 =====
export const listPlugins = () => api.get('/api/plugins')
export const getPlugin = (id: string) => api.get(`/api/plugins/${id}`)
export const installPlugin = (body: any) => api.post('/api/plugins', body)
export const uninstallPlugin = (id: string) => api.delete(`/api/plugins/${id}`)
export const togglePlugin = (id: string) => api.post(`/api/plugins/${id}/toggle`)

// ===== 人格预设 =====
export const listPersonas = () => api.get('/api/personas')
export const getPersona = (id: string) => api.get(`/api/personas/${id}`)
export const createPersona = (body: any) => api.post('/api/personas', body)
export const updatePersona = (id: string, body: any) => api.put(`/api/personas/${id}`, body)
export const deletePersona = (id: string) => api.delete(`/api/personas/${id}`)
export const togglePersona = (id: string) => api.post(`/api/personas/${id}/toggle`)

// ===== 会话 =====
export const listSessions = () => api.get('/api/sessions')
export const getSession = (id: string) => api.get(`/api/sessions/${id}`)
export const deleteSession = (id: string) => api.delete(`/api/sessions/${id}`)
export const deleteAllSessions = () => api.delete('/api/sessions')
export const getSessionHistory = (id: string) => api.get(`/api/sessions/${id}/history`)

// ===== 消息历史 =====
export const listHistory = () => api.get('/api/history')
export const getMessage = (id: string) => api.get(`/api/history/${id}`)
export const deleteMessage = (id: string) => api.delete(`/api/history/${id}`)

// ===== 设置 =====
export const listSettings = () => api.get('/api/settings')
export const updateSettings = (body: any) => api.put('/api/settings', body)
export const getSetting = (key: string) => api.get(`/api/settings/${key}`)
export const updateSetting = (key: string, body: any) => api.put(`/api/settings/${key}`, body)

// ===== 日志 =====
export const getLogs = () => api.get('/api/logs')
