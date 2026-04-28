<template>
  <div>
    <el-card>
      <template #header>
        <div class="card-header">
          <span>会话管理</span>
          <el-button type="primary" @click="refresh">刷新</el-button>
        </div>
      </template>
      <el-table :data="sessions" v-loading="loading">
        <el-table-column prop="id" label="ID" width="220" />
        <el-table-column prop="platform" label="平台" width="100" />
        <el-table-column prop="session_name" label="会话名称" />
        <el-table-column prop="message_count" label="消息数" width="90" />
        <el-table-column prop="last_active" label="最后活跃" width="160" />
        <el-table-column label="操作" width="120">
          <template #default="scope">
            <el-button link type="primary" @click="viewHistory(scope.row.id)">
              查看
            </el-button>
            <el-button link type="danger" @click="deleteSession(scope.row.id)">
              删除
            </el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { api } from '@/api/client'
import { ElMessage, ElMessageBox } from 'element-plus'

const sessions = ref<any[]>([])
const loading = ref(false)

const refresh = async () => {
  loading.value = true
  try {
    const { data } = await api.get('/api/sessions')
    sessions.value = data
  } catch (e) {
    ElMessage.error('加载失败')
  } finally {
    loading.value = false
  }
}

const viewHistory = (id: string) => {
  window.open(`#/history?session=${id}`, '_blank')
}

const deleteSession = async (id: string) => {
  try {
    await ElMessageBox.confirm('确定要删除该会话吗？', '警告', { type: 'warning' })
    await api.delete(`/api/sessions/${id}`)
    ElMessage.success('已删除')
    refresh()
  } catch (e: any) {
    if (e !== 'cancel') ElMessage.error('删除失败')
  }
}

onMounted(refresh)
</script>

<style scoped>
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
</style>
