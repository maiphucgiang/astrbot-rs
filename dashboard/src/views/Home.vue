<template>
  <div class="home">
    <PageHeader title="概览" subtitle="AstrBot 运行状态总览" />
    
    <el-row :gutter="20">
      <el-col :span="6">
        <StatCard 
          title="运行状态" 
          :value="statusText"
          :valueClass="statusHealthy ? 'success' : 'danger'"
          icon="CircleCheck"
          subtitle="系统健康度"
        />
      </el-col>
      <el-col :span="6">
        <StatCard 
          title="消息总数" 
          :value="stats?.total_messages || 0"
          icon="ChatLineRound"
          subtitle="累计处理消息"
        />
      </el-col>
      <el-col :span="6">
        <StatCard 
          title="活跃平台" 
          :value="platforms.length"
          icon="Connection"
          subtitle="已启用适配器"
        />
      </el-col>
      <el-col :span="6">
        <StatCard 
          title="活跃模型" 
          :value="providers.length"
          icon="Cpu"
          subtitle="已配置提供商"
        />
      </el-col>
    </el-row>

    <el-row :gutter="20" class="mt-20">
      <el-col :span="12">
        <el-card>
          <template #header>
            <div class="card-header">
              <span>平台状态</span>
              <el-button type="primary" size="small" @click="loadData">
                <el-icon><Refresh /></el-icon>刷新
              </el-button>
            </div>
          </template>
          <DataLoader :loading="loading" :data="platforms">
            <template #default="{ data }">
              <el-table :data="data" style="width: 100%">
                <el-table-column prop="name" label="平台" />
                <el-table-column prop="enabled" label="状态" width="80">
                  <template #default="scope">
                    <el-tag :type="scope.row.enabled ? 'success' : 'info'" size="small">
                      {{ scope.row.enabled ? '启用' : '禁用' }}
                    </el-tag>
                  </template>
                </el-table-column>
                <el-table-column prop="type" label="类型" width="100" />
              </el-table>
            </template>
          </DataLoader>
        </el-card>
      </el-col>
      <el-col :span="12">
        <el-card>
          <template #header>
            <div class="card-header">
              <span>模型提供商</span>
              <el-button type="primary" size="small" @click="loadData">
                <el-icon><Refresh /></el-icon>刷新
              </el-button>
            </div>
          </template>
          <DataLoader :loading="loading" :data="providers">
            <template #default="{ data }">
              <el-table :data="data" style="width: 100%">
                <el-table-column prop="name" label="提供商" />
                <el-table-column prop="enabled" label="状态" width="80">
                  <template #default="scope">
                    <el-tag :type="scope.row.enabled ? 'success' : 'info'" size="small">
                      {{ scope.row.enabled ? '启用' : '禁用' }}
                    </el-tag>
                  </template>
                </el-table-column>
                <el-table-column prop="model" label="模型" />
              </el-table>
            </template>
          </DataLoader>
        </el-card>
      </el-col>
    </el-row>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, computed } from 'vue'
import { getStatus, listProviders, listPlatforms } from '@/api/client'
import StatCard from '@/components/StatCard.vue'
import PageHeader from '@/components/PageHeader.vue'
import DataLoader from '@/components/DataLoader.vue'

const loading = ref(false)
const stats = ref<any>(null)
const providers = ref<any[]>([])
const platforms = ref<any[]>([])

const statusHealthy = computed(() => stats.value?.status === 'running')
const statusText = computed(() => statusHealthy.value ? '正常' : '异常')

async function loadData() {
  loading.value = true
  try {
    const { data: s } = await getStatus()
    stats.value = s
    const { data: p } = await listProviders()
    providers.value = p?.providers || []
    const { data: a } = await listPlatforms()
    platforms.value = a?.platforms || []
  } catch (e) {
    console.error('Failed to load stats:', e)
  } finally {
    loading.value = false
  }
}

onMounted(loadData)
</script>

<style scoped>
.mt-20 {
  margin-top: 20px;
}
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
</style>
