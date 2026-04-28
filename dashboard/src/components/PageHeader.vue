<template>
  <div class="page-header">
    <div class="title-section">
      <h2>{{ title }}</h2>
      <p v-if="subtitle" class="subtitle">{{ subtitle }}</p>
    </div>
    <div class="actions">
      <slot name="actions" />
    </div>
  </div>
</template>

<script setup lang="ts">
defineProps<{
  title: string
  subtitle?: string
}>()
</script>

<style scoped>
.page-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 20px;
}
.title-section h2 {
  margin: 0;
  font-size: 20px;
  color: #303133;
}
.subtitle {
  margin: 4px 0 0;
  font-size: 13px;
  color: #909399;
}
.actions {
  display: flex;
  gap: 8px;
}
</style>
