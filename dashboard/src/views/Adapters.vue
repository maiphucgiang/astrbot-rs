<template>
  <div>
    <el-card>
      <template #header>
        <div class="card-header">
          <span>平台适配器</span>
          <el-button type="primary" @click="refresh">刷新</el-button>
        </div>
      </template>
      <el-table :data="adapters" v-loading="loading">
        <el-table-column prop="id" label="ID" width="120" />
        <el-table-column prop="name" label="平台" />
        <el-table-column prop="connection_mode" label="连接模式" width="120" />
        <el-table-column prop="enabled" label="状态" width="100">
          <template #default="scope">
            <el-switch
              v-model="scope.row.enabled"
              @change="(val: boolean) => toggleAdapter(scope.row.id, val)"
            />
          </template>
        </el-table-column>
      </el-table>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { api } from '@/api/client'
import { ElMessage } from 'element-plus'

const adapters = ref<any[]>([])
const loading = ref(false)

const refresh = async () => {
  loading.value = true
  try {
    const { data } = await api.get('/api/adapters')
    adapters.value = data
  } catch (e) {
    ElMessage.error('加载失败')
  } finally {
    loading.value = false
  }
}

const toggleAdapter = async (id: string, enabled: boolean) => {
  try {
    await api.post(`/api/adapters/${id}/toggle`, { enabled })
    ElMessage.success(enabled ? '已启用' : '已禁用')
  } catch (e) {
    ElMessage.error('操作失败')
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
