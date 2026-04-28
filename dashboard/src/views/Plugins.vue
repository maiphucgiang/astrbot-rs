<template>
  <div>
    <el-card>
      <template #header>
        <div class="card-header">
          <span>插件管理</span>
          <el-button type="primary">安装插件</el-button>
        </div>
      </template>
      <el-empty description="插件列表加载中..." />
    </el-card>
  </div>
</template>

<script setup lang="ts">
// Plugins view - TODO
</script>

<style scoped>
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
</style>
