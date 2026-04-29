<template>
  <el-container class="layout">
    <el-aside width="200px" class="sidebar">
      <div class="logo">AstrBot</div>
      <el-menu
        :default-active="$route.path"
        router
        class="menu"
        background-color="#1a1a2e"
        text-color="#b8c5d6"
        active-text-color="#409EFF"
      >
        <el-menu-item index="/">
          <el-icon><HomeFilled /></el-icon>
          <span>概览</span>
        </el-menu-item>
        <el-menu-item index="/providers">
          <el-icon><Connection /></el-icon>
          <span>模型提供商</span>
        </el-menu-item>
        <el-menu-item index="/adapters">
          <el-icon><ChatDotRound /></el-icon>
          <span>平台适配器</span>
        </el-menu-item>
        <el-menu-item index="/plugins">
          <el-icon><Box /></el-icon>
          <span>插件</span>
        </el-menu-item>
        <el-menu-item index="/history">
          <el-icon><Clock /></el-icon>
          <span>消息历史</span>
        </el-menu-item>
        <el-menu-item index="/chat">
          <el-icon><ChatDotRound /></el-icon>
          <span>WebChat</span>
        </el-menu-item>
        <el-menu-item index="/config">
          <el-icon><Setting /></el-icon>
          <span>配置</span>
        </el-menu-item>
      </el-menu>
    </el-aside>
    <el-container>
      <el-header class="header">
        <h2>{{ $route.meta.title || 'AstrBot Dashboard' }}</h2>
      </el-header>
      <el-main class="main">
        <router-view />
      </el-main>
    </el-container>
  </el-container>
</template>

<script setup lang="ts">
// Layout component
</script>

<style scoped>
.layout {
  height: 100vh;
}
.sidebar {
  background: #1a1a2e;
}
.logo {
  height: 60px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  font-size: 20px;
  font-weight: bold;
  border-bottom: 1px solid #2a2a3e;
}
.menu {
  border-right: none;
}
.header {
  background: #fff;
  border-bottom: 1px solid #e4e7ed;
  display: flex;
  align-items: center;
}
.main {
  background: #f5f7fa;
  padding: 20px;
}
</style>
