<template>
  <div>
    <PageHeader title="平台适配器" subtitle="管理消息平台连接">
      <template #actions>
        <el-button type="primary" @click="refresh">
          <el-icon><Refresh /></el-icon>刷新
        </el-button>
      </template>
    </PageHeader>

    <el-card>
      <DataLoader :loading="loading" :data="adapters" emptyText="暂无适配器配置">
        <template #default="{ data }">
          <el-table :data="data" style="width: 100%">
            <el-table-column prop="id" label="ID" width="120" />
            <el-table-column prop="name" label="平台" />
            <el-table-column prop="type" label="类型" width="120" />
            <el-table-column prop="connection_mode" label="连接模式" width="120" />
            <el-table-column label="状态" width="100">
              <template #default="scope">
                <el-tag :type="scope.row.enabled ? 'success' : 'info'" size="small">
                  {{ scope.row.enabled ? '启用' : '禁用' }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column label="操作" width="180">
              <template #default="scope">
                <el-button link type="primary" @click="editAdapter(scope.row)">
                  编辑
                </el-button>
                <el-button link type="danger" @click="deleteAdapter(scope.row)">
                  删除
                </el-button>
              </template>
            </el-table-column>
          </el-table>
        </template>
      </DataLoader>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { listPlatforms, updatePlatform, deleteProvider } from '@/api/client'
import PageHeader from '@/components/PageHeader.vue'
import DataLoader from '@/components/DataLoader.vue'

const adapters = ref<any[]>([])
const loading = ref(false)

const refresh = async () => {
  loading.value = true
  try {
    const { data } = await listPlatforms()
    adapters.value = data?.platforms || []
  } catch (e) {
    ElMessage.error('加载失败')
  } finally {
    loading.value = false
  }
}

const editAdapter = (adapter: any) => {
  ElMessage.info(`编辑适配器: ${adapter.name}（待实现）`)
}

const deleteAdapter = async (adapter: any) => {
  try {
    await ElMessageBox.confirm(
      `确定删除适配器 "${adapter.name}" 吗？`,
      '确认删除',
      { type: 'warning' }
    )
    await deleteProvider(adapter.id)
    ElMessage.success('删除成功')
    refresh()
  } catch (e) {
    // Cancelled
  }
}

onMounted(refresh)
</script>
