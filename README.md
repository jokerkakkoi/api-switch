# Anthropic → OpenAI 协议网关

[![Rust](https://img.shields.io/badge/rust-stable-blue.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

基于 Rust 的高性能 HTTP 网关，将 [Anthropic Messages API](https://docs.anthropic.com/en/api/messages) 请求转换为 [OpenAI Chat Completions API](https://platform.openai.com/docs/api-reference/chat) 格式，转发至 OpenAI 兼容的后端，并将响应转换回 Anthropic 格式。支持流式（SSE）和非流式两种模式。

## 特性

- ✅ 完整的 Anthropic Messages API 支持
- ✅ 流式（SSE）和非流式请求
- ✅ 多令牌路由（不同令牌转发到不同后端）
- ✅ 请求头透传（支持 traceId 等追踪头）
- ✅ 图片内容转换
- ✅ 系统提示词转换
- ✅ 健康检查端点
- ✅ 详细的错误响应

## 快速开始

### 前置条件

- Rust 工具链（2024 edition）
- 一个 OpenAI 兼容的后端地址

### 配置文件

创建 `config.yaml` **(重要，必须和exe同文件夹下)**：

```yaml
app_key: "1******1"
app_sign: "f*********8"
openai_base_url: http://******:***/******/ # url不要带/v1/chat/completions
port: 3000
```

### 运行

```bash
export CONFIG_PATH="./config.yaml"            # 默认: ./config.yaml
cargo run
```

服务监听 `http://[IP_ADDRESS]`。

### 测试请求

**非流式请求：**
```bash
curl -X POST http://localhost:3000/v1/messages \
  -H "Authorization: Bearer sk-ant-your-token" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -d '{
    "model": "claude-sonnet-4-20250514",
    "messages": [{"role": "user", "content": "Hello"}],
    "max_tokens": 1024
  }'
```

**流式请求：**
```bash
curl -X POST http://localhost:3000/v1/messages \
  -H "Authorization: Bearer sk-ant-your-token" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -d '{
    "model": "claude-sonnet-4-20250514",
    "messages": [{"role": "user", "content": "Hello"}],
    "max_tokens": 1024,
    "stream": true
  }'
```

**健康检查：**
```bash
curl http://localhost:3000/health
```

## API

| 方法 | 路径 | 说明 |
|--------|------|-------------|
| `POST` | `/v1/messages` | Anthropic → OpenAI 协议转换 |
| `GET`  | `/health` | 健康检查（主要为了测试网络） |

## 工作原理

```
客户端（Anthropic 格式）
      │
      ▼
POST /v1/messages
      │
      ├─ 1. 从 Authorization 头提取 Bearer token
      ├─ 2. 在配置中查找对应的后端凭证（App-Key, App-Sign）
      ├─ 3. 反序列化 Anthropic 请求体
      ├─ 4. 转换 Anthropic 请求 → OpenAI 请求
      ├─ 5. 转发到 {BASE_URL}/v1/chat/completions
      │      请求头: App-Key + App-Sign + 原始头（移除 Authorization）
      ├─ 6. 转换 OpenAI 响应 → Anthropic 响应
      └─ 7. 返回客户端
```

## 协议映射

### 请求：Anthropic → OpenAI

| Anthropic | OpenAI | 备注 |
|-----------|--------|-------|
| `model` | `model` | 直接透传 |
| `system`（字符串或块） | `messages[0]`（role: system） | 追加到消息列表头部 |
| `messages[].role` | `messages[].role` | 直接透传（user/assistant） |
| `messages[].content`（文本） | `messages[].content` | 字符串→字符串，块→块 |
| `messages[].content`（图片） | `messages[].content` | `image` → `image_url`（data URL） |
| `max_tokens` | `max_tokens` | 直接透传 |
| `stop_sequences` | `stop` | 字段重命名 |
| `temperature` | `temperature` | 直接透传 |
| `top_p` | `top_p` | 直接透传 |
| `top_k` | — | 丢弃（OpenAI 无对应字段） |
| `stream` | `stream` | 直接透传 |

### 响应：OpenAI → Anthropic

| OpenAI | Anthropic | 备注 |
|--------|-----------|-------|
| `id` | `id` | 直接透传 |
| `model` | `model` | 直接透传 |
| `choices[0].message.role` | `role` | 固定 `assistant` |
| `choices[0].message.content` | `content[0]`（text block） | 字符串 → 内容块数组 |
| `choices[0].finish_reason` | `stop_reason` | `stop`→`end_turn`, `length`→`max_tokens`, `tool_calls`→`tool_use` |
| `usage` | `usage` | 字段映射（prompt_tokens→input_tokens 等） |

### SSE 流式事件

| Anthropic 事件 | 触发条件 |
|----------------|---------|
| `message_start` | 首个 delta chunk |
| `content_block_start` | 首个内容 delta |
| `content_block_delta` | 后续内容 delta |
| `content_block_stop` | 内容块结束 |
| `message_delta` | 最后一个 delta（含 stop_reason） |
| `message_stop` | 流结束 |
| `ping` | 心跳保活 |

## 错误格式

所有错误均返回 Anthropic 风格的错误结构：

```json
{
  "type": "error",
  "error": {
    "type": "authentication_error",
    "message": "Invalid bearer token"
  }
}
```

## 配置参考

### `config.yaml`

```yaml
app_key: "<app_key>"
app_sign: "<app_sign>"
# url不要带/v1/chat/completions
openai_base_url: http://[IP_ADDRESS]
port: 3000
```

## 开发

```bash
# 构建
cargo build -r

# 运行全部测试
cargo test

# 运行指定模块测试
cargo test -- config
cargo test -- error
cargo test -- anthropic
cargo test -- openai
cargo test -- transform::request
cargo test -- transform::response
cargo test -- transform::stream

# 显示测试输出
cargo test -- --nocapture
```

## 项目结构

```
src/
├── main.rs           # 入口点，路由注册，AxumState 初始化
├── config.rs         # YAML 配置加载，令牌→凭证查找
├── error.rs          # AppError 枚举（27 种错误变体）
├── handler.rs        # /v1/messages 和 /health 处理器
├── anthropic/        # Anthropic 协议定义
│   └── mod.rs        # MessagesRequest, MessagesResponse, SSE 事件
├── openai/           # OpenAI 协议定义
│   └── mod.rs        # ChatRequest, ChatResponse, SSE 块
└── transform/        # 协议转换
    ├── mod.rs        # 模块导出
    ├── request.rs    # Anthropic → OpenAI 请求转换
    ├── response.rs   # OpenAI → Anthropic 响应转换
    └── stream.rs     # SSE 流式转换（状态机）
```

## 许可证

MIT
