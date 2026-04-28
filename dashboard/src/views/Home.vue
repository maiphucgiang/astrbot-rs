<template>
  <div class="home">
    <el-row :gutter="20">
      <el-col :span="6">
        <el-card>
          <template #header>
            <div class="card-header">
              <span>运行状态</span>
            </div>
          </template>
          <div class="stat-value" :class="{ healthy: stats?.healthy }">
            {{ stats?.healthy ? '正常' : '异常' }}
          </div>
        </el-card>
      </el-col>
      <el-col :span="6">
        <el-card>
          <template #header>
            <div class="card-header">
              <span>消息总数</span>
            </div>
          </template>
          <div class="stat-value">{{ stats?.total_messages || 0 }}</div>
        </el-card>
      </el-col>
      <el-col :span="6">
        <el-card>
          <template #header>
            <div class="card-header">
              <span>活跃平台</span>
            </div>
          </template>
          <div class="stat-value">{{ stats?.active_adapters || 0 }}</div>
        </el-card>
      </el-col>
      <el-col :span="6">
        <el-card>
          <template #header>
            <div class="card-header">
              <span>活跃模型</span>
            </div>
          </template>
          <div class="stat-value">{{ stats?.active_providers || 0 }}</div>
        </el-card>
      </el-col>
    </el-row>

    <el-row :gutter="20" class="mt-20">
      <el-col :span="12">
        <el-card>
          <template #header>平台状态</template>
          <el-table :data="adapters" style="width: 100%">
            <el-table-column prop="name" label="平台" />
            <el-table-column prop="enabled" label="状态">
              <template #default="scope">
                <el-tag :type="scope.row.enabled ? 'success' : 'info'">
                  {{ scope.row.enabled ? '启用' : '禁用' }}
                </el-tag>
              </template>
            </el-table-column>
          </el-table>
        </el-card>
      </el-col>
      <el-col :span="12">
        <el-card>
          <template #header>模型提供商</template>
          <el-table :data="providers" style="width: 100%">
            <el-table-column prop="name" label="提供商" />
            <el-table-column prop="enabled" label="状态">
              <template #default="scope">
                <el-tag :type="scope.row.enabled ? 'success' : 'info'">
                  {{ scope.row.enabled ? '启用' : '禁用' }}
                </el-tag>
              </template>
            </el-table-column>
          </el-table>
        </el-card>
      </el-col>
    </el-row>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { api } from '@/api/client'

const stats = ref<any>(null)
const adapters = ref<any[]>([])
const providers = ref<any[]>([])

onMounted(async () => {
  try {
    const { data: s } = await api.get('/api/stats')
    stats.value = s
    const { data: a } = await api.get('/api/adapters')
    adapters.value = a
    const { data: p } = await api.get('/api/providers')
    providers.value = p
  } catch (e) {
    console.error('Failed to load stats:', e)
  }
})
</script>

<style scoped>
.stat-value {
  font-size: 32px;
  font-weight: bold;
  color: #f56c6c;
}
.stat-value.healthy {
  color: #67c23a;
}
.mt-20 {
  margin-top: 20px;
}
</style>
