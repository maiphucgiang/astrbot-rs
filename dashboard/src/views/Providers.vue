<template>
  <div>
    <el-card>
      <template #header>
        <div class="card-header">
          <span>模型提供商</span>
          <el-button type="primary" @click="refresh">刷新</el-button>
        </div>
      </template>
      <el-table :data="providers" v-loading="loading">
        <el-table-column prop="id" label="ID" width="120" />
        <el-table-column prop="name" label="名称" />
        <el-table-column prop="type" label="类型" width="120" />
        <el-table-column prop="enabled" label="状态" width="100">
          <template #default="scope">
            <el-switch
              v-model="scope.row.enabled"
              @change="(val: boolean) => toggleProvider(scope.row.id, val)"
            />
          </template>
        </el-table-column>
        <el-table-column label="操作" width="120">
          <template #default="scope">
            <el-button link type="primary" @click="editProvider(scope.row)">
              编辑
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
import { ElMessage } from 'element-plus'

const providers = ref<any[]>([])
const loading = ref(false)

const refresh = async () => {
  loading.value = true
  try {
    const { data } = await api.get('/api/providers')
    providers.value = data
  } catch (e) {
    ElMessage.error('加载失败')
  } finally {
    loading.value = false
  }
}

const toggleProvider = async (id: string, enabled: boolean) => {
  try {
    await api.post(`/api/providers/${id}/toggle`, { enabled })
    ElMessage.success(enabled ? '已启用' : '已禁用')
  } catch (e) {
    ElMessage.error('操作失败')
  }
}

const editProvider = (provider: any) => {
  // TODO: open edit dialog
  console.log('Edit provider:', provider)
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
