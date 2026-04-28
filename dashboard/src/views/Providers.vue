<template>
  <div>
    <PageHeader title="模型提供商" subtitle="管理 LLM 提供商配置">
      <template #actions>
        <el-button type="primary" @click="refresh">
          <el-icon><Refresh /></el-icon>刷新
        </el-button>
      </template>
    </PageHeader>

    <el-card>
      <DataLoader :loading="loading" :data="providers" emptyText="暂无提供商配置">
        <template #default="{ data }">
          <el-table :data="data" style="width: 100%">
            <el-table-column prop="id" label="ID" width="120" />
            <el-table-column prop="name" label="名称" />
            <el-table-column prop="type" label="类型" width="120" />
            <el-table-column prop="model" label="模型" />
            <el-table-column label="状态" width="100">
              <template #default="scope">
                <el-tag :type="scope.row.enabled ? 'success' : 'info'" size="small">
                  {{ scope.row.enabled ? '启用' : '禁用' }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column label="操作" width="180">
              <template #default="scope">
                <el-button link type="primary" @click="testProvider(scope.row)">
                  测试
                </el-button>
                <el-button link type="primary" @click="editProvider(scope.row)">
                  编辑
                </el-button>
                <el-button link type="danger" @click="deleteProvider(scope.row)">
                  删除
                </el-button>
              </template>
            </el-table-column>
          </el-table>
        </template>
      </DataLoader>
    </el-card>

    <!-- Test Result Dialog -->
    <el-dialog v-model="testDialogVisible" title="连通性测试" width="400px">
      <div v-if="testLoading" class="test-loading">
        <el-skeleton :rows="3" animated />
      </div>
      <div v-else-if="testResult">
        <el-result
          :icon="testResult.success ? 'success' : 'error'"
          :title="testResult.success ? '连通正常' : '连接失败'"
          :sub-title="testResult.message || `延迟: ${testResult.latency_ms}ms`"
        />
      </div>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { listProviders, testProvider as apiTestProvider, deleteProvider as apiDeleteProvider } from '@/api/client'
import PageHeader from '@/components/PageHeader.vue'
import DataLoader from '@/components/DataLoader.vue'

const providers = ref<any[]>([])
const loading = ref(false)
const testDialogVisible = ref(false)
const testLoading = ref(false)
const testResult = ref<any>(null)

const refresh = async () => {
  loading.value = true
  try {
    const { data } = await listProviders()
    providers.value = data?.providers || []
  } catch (e) {
    ElMessage.error('加载失败')
  } finally {
    loading.value = false
  }
}

const testProvider = async (provider: any) => {
  testDialogVisible.value = true
  testLoading.value = true
  testResult.value = null
  try {
    const { data } = await apiTestProvider(provider.id)
    testResult.value = data
  } catch (e) {
    testResult.value = { success: false, message: '测试请求失败' }
  } finally {
    testLoading.value = false
  }
}

const editProvider = (provider: any) => {
  ElMessage.info(`编辑提供商: ${provider.name}（待实现）`)
}

const deleteProvider = async (provider: any) => {
  try {
    await ElMessageBox.confirm(
      `确定删除提供商 "${provider.name}" 吗？`,
      '确认删除',
      { type: 'warning' }
    )
    await apiDeleteProvider(provider.id)
    ElMessage.success('删除成功')
    refresh()
  } catch (e) {
    // Cancelled
  }
}

onMounted(refresh)
</script>

<style scoped>
.test-loading {
  padding: 20px;
}
</style>
