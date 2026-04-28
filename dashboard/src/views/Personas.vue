<template>
  <div>
    <el-card>
      <template #header>
        <div class="card-header">
          <span>人格预设</span>
          <el-button type="primary" @click="refresh">刷新</el-button>
        </div>
      </template>
      <el-table :data="personas" v-loading="loading">
        <el-table-column prop="id" label="ID" width="140" />
        <el-table-column prop="name" label="名称" />
        <el-table-column prop="description" label="描述" />
        <el-table-column prop="active" label="状态" width="100">
          <template #default="scope">
            <el-tag :type="scope.row.active ? 'success' : 'info'">
              {{ scope.row.active ? '启用' : '禁用' }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="操作" width="180">
          <template #default="scope">
            <el-button link type="primary" @click="activate(scope.row.id)">
              {{ scope.row.active ? '停用' : '启用' }}
            </el-button>
            <el-button link type="primary" @click="editPersona(scope.row)">
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

const personas = ref<any[]>([])
const loading = ref(false)

const refresh = async () => {
  loading.value = true
  try {
    const { data } = await api.get('/api/personas')
    personas.value = data
  } catch (e) {
    ElMessage.error('加载失败')
  } finally {
    loading.value = false
  }
}

const activate = async (id: string) => {
  try {
    await api.post(`/api/personas/${id}/toggle`)
    ElMessage.success('状态已切换')
    refresh()
  } catch (e) {
    ElMessage.error('操作失败')
  }
}

const editPersona = (p: any) => {
  console.log('Edit persona:', p)
  // TODO: open drawer dialog
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
