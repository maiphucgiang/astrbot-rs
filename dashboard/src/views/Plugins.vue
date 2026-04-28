<template>
  <div>
    <PageHeader title="插件管理" subtitle="安装、启用和管理 AstrBot 插件">
      <template #actions>
        <el-button type="primary" @click="installDialogVisible = true">
          <el-icon><Plus /></el-icon>安装插件
        </el-button>
        <el-button @click="refresh">
          <el-icon><Refresh /></el-icon>刷新
        </el-button>
      </template>
    </PageHeader>

    <el-card>
      <DataLoader :loading="loading" :data="plugins" emptyText="暂无插件">
        <template #default="{ data }">
          <el-table :data="data" style="width: 100%">
            <el-table-column prop="id" label="ID" width="140" />
            <el-table-column prop="name" label="名称" />
            <el-table-column prop="version" label="版本" width="100" />
            <el-table-column prop="description" label="描述" />
            <el-table-column label="状态" width="100">
              <template #default="scope">
                <el-tag :type="scope.row.enabled ? 'success' : 'info'" size="small">
                  {{ scope.row.enabled ? '启用' : '禁用' }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column label="操作" width="180">
              <template #default="scope">
                <el-button link type="primary" @click="togglePlugin(scope.row)">
                  {{ scope.row.enabled ? '停用' : '启用' }}
                </el-button>
                <el-button link type="danger" @click="uninstallPlugin(scope.row)">
                  卸载
                </el-button>
              </template>
            </el-table-column>
          </el-table>
        </template>
      </DataLoader>
    </el-card>

    <!-- Install Dialog -->
    <el-dialog v-model="installDialogVisible" title="安装插件" width="500px">
      <el-form :model="installForm" label-width="80px">
        <el-form-item label="来源">
          <el-radio-group v-model="installForm.source">
            <el-radio label="market">插件市场</el-radio>
            <el-radio label="url">URL</el-radio>
            <el-radio label="local">本地路径</el-radio>
          </el-radio-group>
        </el-form-item>
        <el-form-item label="标识">
          <el-input v-model="installForm.identifier" placeholder="插件 ID 或 URL" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="installDialogVisible = false">取消</el-button>
        <el-button type="primary" @click="confirmInstall" :loading="installLoading">安装</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, reactive } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { listPlugins, togglePlugin as apiTogglePlugin, uninstallPlugin as apiUninstallPlugin, installPlugin } from '@/api/client'
import PageHeader from '@/components/PageHeader.vue'
import DataLoader from '@/components/DataLoader.vue'

const plugins = ref<any[]>([])
const loading = ref(false)
const installDialogVisible = ref(false)
const installLoading = ref(false)
const installForm = reactive({
  source: 'market',
  identifier: ''
})

const refresh = async () => {
  loading.value = true
  try {
    const { data } = await listPlugins()
    plugins.value = data?.plugins || []
  } catch (e) {
    ElMessage.error('加载失败')
  } finally {
    loading.value = false
  }
}

const togglePlugin = async (plugin: any) => {
  try {
    await apiTogglePlugin(plugin.id)
    ElMessage.success('状态已切换')
    refresh()
  } catch (e) {
    ElMessage.error('操作失败')
  }
}

const uninstallPlugin = async (plugin: any) => {
  try {
    await ElMessageBox.confirm(
      `确定卸载插件 "${plugin.name}" 吗？`,
      '确认卸载',
      { type: 'warning' }
    )
    await apiUninstallPlugin(plugin.id)
    ElMessage.success('卸载成功')
    refresh()
  } catch (e) {
    // Cancelled
  }
}

const confirmInstall = async () => {
  installLoading.value = true
  try {
    await installPlugin({
      source: installForm.source,
      identifier: installForm.identifier
    })
    ElMessage.success('安装成功')
    installDialogVisible.value = false
    refresh()
  } catch (e) {
    ElMessage.error('安装失败')
  } finally {
    installLoading.value = false
  }
}

onMounted(refresh)
</script>
