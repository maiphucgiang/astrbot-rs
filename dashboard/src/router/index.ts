import { createRouter, createWebHistory } from 'vue-router'

const routes = [
  {
    path: '/',
    name: 'Home',
    component: () => import('@/views/Home.vue'),
    meta: { title: '概览' }
  },
  {
    path: '/providers',
    name: 'Providers',
    component: () => import('@/views/Providers.vue'),
    meta: { title: '模型提供商' }
  },
  {
    path: '/adapters',
    name: 'Adapters',
    component: () => import('@/views/Adapters.vue'),
    meta: { title: '平台适配器' }
  },
  {
    path: '/plugins',
    name: 'Plugins',
    component: () => import('@/views/Plugins.vue'),
    meta: { title: '插件' }
  },
  {
    path: '/history',
    name: 'History',
    component: () => import('@/views/History.vue'),
    meta: { title: '消息历史' }
  },
  {
    path: '/config',
    name: 'Config',
    component: () => import('@/views/Config.vue'),
    meta: { title: '配置管理' }
  }
]

const router = createRouter({
  history: createWebHistory(),
  routes
})

export default router
