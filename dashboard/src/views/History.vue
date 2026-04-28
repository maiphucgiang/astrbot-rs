<template>
  <div>
    <el-card>
      <template #header>
        <div class="card-header">
          <span>消息历史</span>
          <el-input
            v-model="searchQuery"
            placeholder="搜索消息..."
            style="width: 300px"
            clearable
          />
        </div>
      </template>
      <el-table :data="messages" v-loading="loading">
        <el-table-column prop="timestamp" label="时间" width="180" />
        <el-table-column prop="platform" label="平台" width="120" />
        <el-table-column prop="sender" label="发送者" width="150" />
        <el-table-column prop="content" label="内容" show-overflow-tooltip />
      </el-table>
      <el-pagination
        v-model:current-page="page"
        v-model:page-size="pageSize"
        :total="total"
        layout="prev, pager, next"
        class="pagination"
        @current-change="loadMessages"
      />
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { api } from '@/api/client'

const messages = ref<any[]>([])
const loading = ref(false)
const searchQuery = ref('')
const page = ref(1)
const pageSize = ref(20)
const total = ref(0)

const loadMessages = async () => {
  loading.value = true
  try {
    const { data } = await api.get('/api/history', {
      params: {
        page: page.value,
        limit: pageSize.value,
        q: searchQuery.value
      }
    })
    messages.value = data.messages
    total.value = data.total
  } catch (e) {
    console.error('Failed to load messages:', e)
  } finally {
    loading.value = false
  }
}

onMounted(loadMessages)
</script>

<style scoped>
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
.pagination {
  margin-top: 20px;
  justify-content: flex-end;
}
</style>
