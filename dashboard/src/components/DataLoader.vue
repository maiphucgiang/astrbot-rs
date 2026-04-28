<template>
  <el-empty v-if="!loading && (!data || data.length === 0)" :description="emptyText" />
  <el-skeleton v-else-if="loading" :rows="5" animated />
  <slot v-else :data="data" />
</template>

<script setup lang="ts">
withDefaults(defineProps<{
  loading: boolean
  data?: any[]
  emptyText?: string
}>(), {
  emptyText: '暂无数据',
  data: () => []
})
</script>
