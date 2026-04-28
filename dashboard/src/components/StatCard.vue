<template>
  <el-card>
    <template #header>
      <div class="card-header">
        <el-icon v-if="icon" :size="20"><component :is="icon" /></el-icon>
        <span>{{ title }}</span>
      </div>
    </template>
    <div class="stat-value" :class="valueClass">
      <slot name="value">{{ value }}</slot>
    </div>
    <div v-if="subtitle" class="stat-subtitle">{{ subtitle }}</div>
  </el-card>
</template>

<script setup lang="ts">
defineProps<{
  title: string
  value?: string | number
  subtitle?: string
  icon?: string
  valueClass?: string
}>()
</script>

<style scoped>
.card-header {
  display: flex;
  align-items: center;
  gap: 8px;
}
.stat-value {
  font-size: 32px;
  font-weight: bold;
  color: #303133;
}
.stat-value.success { color: #67c23a; }
.stat-value.warning { color: #e6a23c; }
.stat-value.danger { color: #f56c6c; }
.stat-subtitle {
  margin-top: 8px;
  font-size: 12px;
  color: #909399;
}
</style>
