<template>
  <div>
    <el-card>
      <template #header>
        <div class="card-header">
          <span>配置管理</span>
          <el-button type="primary" @click="saveConfig">保存</el-button>
        </div>
      </template>
      <el-form :model="config" label-width="120px">
        <el-form-item label="Bot名称">
          <el-input v-model="config.bot_name" />
        </el-form-item>
        <el-form-item label="管理员ID">
          <el-input v-model="config.admin_id" />
        </el-form-item>
        <el-form-item label="日志级别">
          <el-select v-model="config.log_level">
            <el-option label="DEBUG" value="debug" />
            <el-option label="INFO" value="info" />
            <el-option label="WARN" value="warn" />
            <el-option label="ERROR" value="error" />
          </el-select>
        </el-form-item>
      </el-form>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { api } from '@/api/client'
import { ElMessage } from 'element-plus'

const config = ref<any>({})

const loadConfig = async () => {
  try {
    const { data } = await api.get('/api/config')
    config.value = data
  } catch (e) {
    console.error('Failed to load config:', e)
  }
}

const saveConfig = async () => {
  try {
    await api.post('/api/config', config.value)
    ElMessage.success('配置已保存')
  } catch (e) {
    ElMessage.error('保存失败')
  }
}

onMounted(loadConfig)
</script>

<style scoped>
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
</style>
