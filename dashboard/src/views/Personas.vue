<template>
  <div>
    <PageHeader title="人格预设" subtitle="管理 Bot 人格与对话风格">
      <template #actions>
        <el-button type="primary" @click="createDialogVisible = true">
          <el-icon><Plus /></el-icon>新建人格
        </el-button>
        <el-button @click="refresh">
          <el-icon><Refresh /></el-icon>刷新
        </el-button>
      </template>
    </PageHeader>

    <el-card>
      <DataLoader :loading="loading" :data="personas" emptyText="暂无人格预设">
        <template #default="{ data }">
          <el-table :data="data" style="width: 100%">
            <el-table-column prop="id" label="ID" width="140" />
            <el-table-column prop="name" label="名称" />
            <el-table-column prop="description" label="描述" />
            <el-table-column prop="tags" label="标签" width="120">
              <template #default="scope">
                <el-tag v-for="tag in scope.row.tags || []" :key="tag" size="small" class="tag-item">
                  {{ tag }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column label="状态" width="100">
              <template #default="scope">
                <el-tag :type="scope.row.active ? 'success' : 'info'" size="small">
                  {{ scope.row.active ? '启用' : '禁用' }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column label="操作" width="200">
              <template #default="scope">
                <el-button link type="primary" @click="activate(scope.row.id)">
                  {{ scope.row.active ? '停用' : '启用' }}
                </el-button>
                <el-button link type="primary" @click="editPersona(scope.row)">
                  编辑
                </el-button>
                <el-button link type="danger" @click="deletePersona(scope.row)">
                  删除
                </el-button>
              </template>
            </el-table-column>
          </el-table>
        </template>
      </DataLoader>
    </el-card>

    <!-- Create Persona Dialog -->
    <el-dialog v-model="createDialogVisible" title="新建人格" width="600px">
      <el-form :model="personaForm" label-width="100px">
        <el-form-item label="名称" required>
          <el-input v-model="personaForm.name" placeholder="人格名称" />
        </el-form-item>
        <el-form-item label="描述">
          <el-input v-model="personaForm.description" type="textarea" :rows="2" placeholder="简短描述" />
        </el-form-item>
        <el-form-item label="系统提示词" required>
          <el-input v-model="personaForm.system_prompt" type="textarea" :rows="6" placeholder="系统提示词..." />
        </el-form-item>
        <el-form-item label="标签">
          <el-input v-model="tagInput" placeholder="逗号分隔" @blur="addTags" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="createDialogVisible = false">取消</el-button>
        <el-button type="primary" @click="confirmCreate" :loading="createLoading">创建</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, reactive } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { listPersonas, togglePersona as apiTogglePersona, createPersona as apiCreatePersona, deletePersona as apiDeletePersona } from '@/api/client'
import PageHeader from '@/components/PageHeader.vue'
import DataLoader from '@/components/DataLoader.vue'

const personas = ref<any[]>([])
const loading = ref(false)
const createDialogVisible = ref(false)
const createLoading = ref(false)
const tagInput = ref('')

const personaForm = reactive({
  name: '',
  description: '',
  system_prompt: '',
  tags: [] as string[]
})

const refresh = async () => {
  loading.value = true
  try {
    const { data } = await listPersonas()
    personas.value = data?.personas || []
  } catch (e) {
    ElMessage.error('加载失败')
  } finally {
    loading.value = false
  }
}

const activate = async (id: string) => {
  try {
    await apiTogglePersona(id)
    ElMessage.success('状态已切换')
    refresh()
  } catch (e) {
    ElMessage.error('操作失败')
  }
}

const editPersona = (p: any) => {
  ElMessage.info(`编辑人格: ${p.name}（待实现）`)
}

const deletePersona = async (p: any) => {
  try {
    await ElMessageBox.confirm(
      `确定删除人格 "${p.name}" 吗？`,
      '确认删除',
      { type: 'warning' }
    )
    await apiDeletePersona(p.id)
    ElMessage.success('删除成功')
    refresh()
  } catch (e) {
    // Cancelled
  }
}

const addTags = () => {
  if (tagInput.value) {
    personaForm.tags = tagInput.value.split(',').map(t => t.trim()).filter(Boolean)
  }
}

const confirmCreate = async () => {
  createLoading.value = true
  try {
    await apiCreatePersona({ ...personaForm })
    ElMessage.success('创建成功')
    createDialogVisible.value = false
    Object.assign(personaForm, { name: '', description: '', system_prompt: '', tags: [] })
    tagInput.value = ''
    refresh()
  } catch (e) {
    ElMessage.error('创建失败')
  } finally {
    createLoading.value = false
  }
}

onMounted(refresh)
</script>

<style scoped>
.tag-item {
  margin-right: 4px;
}
</style>
