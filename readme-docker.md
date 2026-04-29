# AstrBot Rust — Docker 部署指南

## 快速开始

### 单容器启动

```bash
# 1. Clone 仓库
git clone https://github.com/Last-emo-boy/astrbot-rs.git
cd astrbot-rs

# 2. 构建镜像
docker build -t astrbot-rs:latest .

# 3. 运行
mkdir -p ./data ./plugins ./logs
docker run -d \
  --name astrbot \
  -p 6185:6185 \
  -v $(pwd)/config.json:/app/config.json:ro \
  -v $(pwd)/data:/app/data \
  -v $(pwd)/plugins:/app/plugins \
  -v $(pwd)/logs:/app/logs \
  -e RUST_LOG=info \
  -e TZ=Asia/Shanghai \
  astrbot-rs:latest

# 4. 查看日志
docker logs -f astrbot
```

### docker-compose（推荐）

```yaml
services:
  astrbot:
    build:
      context: .
      dockerfile: Dockerfile
    image: astrbot-rs:latest
    container_name: astrbot
    restart: unless-stopped
    ports:
      - "6185:6185"
    volumes:
      - ./config.json:/app/config.json:ro
      - astrbot-data:/app/data
      - astrbot-plugins:/app/plugins
      - astrbot-logs:/app/logs
    environment:
      - RUST_LOG=info
      - ASTRBOT_BIND_ADDR=0.0.0.0:6185
      - TZ=Asia/Shanghai
    healthcheck:
      test: ["CMD", "astrbot", "status"]
      interval: 30s
      timeout: 5s
      retries: 3
      start_period: 60s
    deploy:
      resources:
        limits:
          cpus: '2.0'
          memory: 512M
        reservations:
          cpus: '0.5'
          memory: 128M
    networks:
      - astrbot-net

  # Optional: PostgreSQL + pgvector
  # postgres:
  #   image: pgvector/pgvector:pg16
  #   container_name: astrbot-postgres
  #   restart: unless-stopped
  #   environment:
  #     POSTGRES_USER: astrbot
  #     POSTGRES_PASSWORD: changeme
  #     POSTGRES_DB: astrbot
  #   volumes:
  #     - astrbot-pgdata:/var/lib/postgresql/data
  #   networks:
  #     - astrbot-net

  # Optional: Redis
  # redis:
  #   image: redis:7-alpine
  #   container_name: astrbot-redis
  #   restart: unless-stopped
  #   volumes:
  #     - astrbot-redisdata:/data
  #   networks:
  #     - astrbot-net

volumes:
  astrbot-data:
  astrbot-plugins:
  astrbot-logs:
  # astrbot-pgdata:
  # astrbot-redisdata:

networks:
  astrbot-net:
    driver: bridge
```

启动：
```bash
docker compose up -d
docker compose logs -f
```

## Dockerfile 说明

多阶段构建：
- **Stage 1** (builder): `rust:1.75-slim-bookworm`，编译依赖缓存 + 完整构建
- **Stage 2** (runtime): `debian:bookworm-slim`，非 root 用户运行，仅含 ca-certificates + libssl3

镜像体积：~80MB（runtime）

## 环境变量

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `RUST_LOG` | `info` | 日志级别（trace/debug/info/warn/error） |
| `ASTRBOT_BIND_ADDR` | `0.0.0.0:6185` | Dashboard 绑定地址 |
| `TZ` | `UTC` | 时区 |
| `ASTRBOT_CONFIG_PATH` | `/app/config.json` | 配置文件路径 |

## 端口映射

| 端口 | 用途 | 必需 |
|------|------|------|
| 6185 | Dashboard / WebChat | ✅ |
| 3000 | Misskey / webhook | 可选 |
| 3001 | Matrix / DingTalk / WeCom | 可选 |
| 8080 | 微信 bridge / 自定义 | 可选 |

## 数据持久化

| 挂载点 | 内容 | 备份策略 |
|--------|------|----------|
| `/app/data` | SQLite 数据库、会话数据 | 定期 rsync |
| `/app/plugins` | WASM 插件、第三方扩展 | git 版本控制 |
| `/app/logs` | 运行日志 | logrotate |

## 健康检查

```bash
# 容器内
docker exec astrbot astrbot status

# 外部 HTTP
curl http://localhost:6185/api/health
```

## 生产环境 Checklist

1. ✅ 配置文件 config.json 挂载为 read-only（`:ro`）
2. ✅ 数据卷使用 named volume 或 host bind mount
3. ✅ 设置资源限制（memory: 512M / cpu: 2.0）
4. ✅ 启用 restart: unless-stopped
5. ✅ 配置健康检查（30s interval / 5s timeout）
6. ✅ 使用非 root 用户运行容器
7. ✅ 限制容器特权（no-new-privileges）
8. ✅ 日志收集到独立卷，配置 logrotate
9. ✅ 数据库（如使用 PostgreSQL）单独容器 + 定期备份
10. ✅ 使用 GitHub Container Registry（GHCR）自动更新：`ghcr.io/last-emo-boy/astrbot-rs:latest`

## 一键部署脚本

```bash
#!/bin/bash
set -e

REPO="https://github.com/Last-emo-boy/astrbot-rs"
CONFIG_URL="${1:-}"

echo "=== AstrBot Rust Docker 部署 ==="

# Clone
git clone "$REPO" astrbot-rs && cd astrbot-rs

# 下载配置（如果提供 URL）
if [ -n "$CONFIG_URL" ]; then
  curl -L -o config.json "$CONFIG_URL"
else
  cp config.example.json config.json
  echo "⚠️ 使用默认配置，请编辑 config.json 填入 API key"
fi

# 构建 + 启动
docker compose up -d --build

echo "✅ 部署完成"
echo "Dashboard: http://localhost:6185"
echo "Logs: docker compose logs -f"
```

使用：
```bash
./deploy.sh https://your-config-server.com/config.json
```
