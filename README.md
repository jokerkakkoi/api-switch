# Anthropic → OpenAI 协议网关

[![Rust](https://img.shields.io/badge/rust-stable-blue.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

基于 Rust 的高性能 HTTP 网关，将 [Anthropic Messages API](https://docs.anthropic.com/en/api/messages) 和 [OpenAI Chat Completions API](https://platform.openai.com/docs/api-reference/chat) 格式，转发至**多后端 OpenAI 兼容**的服务。支持流式（SSE）和非流式两种模式，全面支持接入各大主流 AI Coding Agent。

> 后端认证采用 **AppKey** 和 **AppSign** 请求头，而非标准 Authorization 头。每个模型独立配置认证凭据和后端地址，请求通过 `model` 字段自动路由。

## 特性

- ✅ 完整的 Anthropic Messages API 支持
- ✅ OpenAI Chat Completions API 透明代理
- ✅ 多模型路由（每个模型独立后端和认证）
- ✅ 流式（SSE）和非流式请求
- ✅ 请求头透传（支持 traceId 等追踪头）
- ✅ 图片内容转换
- ✅ 工具调用（完整 Anthropic ↔ OpenAI 格式转换，流式 tool call 累积）
- ✅ 系统提示词转换
- ✅ 健康检查端点（代理到后端）
- ✅ 后端 HTTP 客户端超时 300s
- ✅ 禁用代理（no_proxy）

## 快速开始

详细的使用说明，包括快速开始、配置说明、AI Coding Agent 配置和常见问题，请查看 [docs/HOWTOUSE.md](docs/HOWTOUSE.md)。

## API

| 方法 | 路径 | 说明 |
|--------|------|-------------|
| `POST` | `/v1/messages` | Anthropic → OpenAI 协议转换 |
| `POST` | `/v1/chat/completions` | OpenAI 透明代理（无协议转换，直接透传） |
| `GET`  | `/health` | 健康检查（代理到后端） |

## 工作原理

### `/v1/messages` — Anthropic → OpenAI 协议转换

```
客户端（Anthropic 格式）
      │
      ▼
POST /v1/messages
      │
      ├─ 1. 解析 Anthropic 请求体，提取 model 字段
      ├─ 2. 根据 model 名称查找对应 ModelConfig（AppKey + AppSign + BaseURL）
      ├─ 3. 转换 Anthropic 请求 → OpenAI 请求
      ├─ 4. 构建转发请求头：App-Key + App-Sign + 原始头（移除跳转头）
      ├─ 5. 转发到 {base_url}/v1/chat/completions
      ├─ 6. 转换 OpenAI 响应 → Anthropic 响应
      │     - 非流式：直接转换 JSON
      │     - 流式：SSE 状态机转换 OpenAI chunk → Anthropic 事件
      └─ 7. 返回客户端
```

### `/v1/chat/completions` — OpenAI 透明代理

```
客户端（OpenAI 格式）
      │
      ▼
POST /v1/chat/completions
      │
      ├─ 1. 提取 model 字段，查找对应 ModelConfig
      ├─ 2. 构建转发请求头：App-Key + App-Sign + 原始头
      ├─ 3. 直接透传请求体到 {base_url}/v1/chat/completions
      ├─ 4. 透传后端响应状态码和响应头
      ├─ 5. 透传响应体（流式/非流式自动检测）
      └─ 6. 返回客户端
```

### `/health` — 健康检查

```
客户端
  │
  ▼
GET /health
  │
  ├─ 1. 转发到第一个模型的 {base_url}/health
  ├─ 2. 透传后端响应状态码和响应体
  └─ 3. 返回客户端
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
models:
  - name: qwen35-397b        # 模型名称，用于路由
    app_key: "<app_key>"     # 后端认证密钥
    app_sign: "<app_sign>"   # 后端认证签名
    base_url: http://host1/  # 后端地址，不要带 /v1/chat/completions
  - name: glm-5
    app_key: "<app_key>"
    app_sign: "<app_sign>"
    base_url: http://host2/
port: 3000                    # 可选，默认 3000
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
├── main.rs           # 入口点，路由注册，端口从配置读取，300s 客户端超时
├── config.rs         # YAML 配置加载（AppConfig: models Vec<ModelConfig>, port），find_model() 查找
├── error.rs          # AppError 枚举（5 种变体），Anthropic 风格错误响应
├── handler.rs        # 路由处理器，AppState，模型解析，SSE 流式
├── anthropic/        # Anthropic 协议定义
│   └── mod.rs        # MessagesRequest, MessagesResponse, SSE 事件, tools
├── openai/           # OpenAI 协议定义
│   └── mod.rs        # ChatRequest, ChatResponse, OpenAISSEChunk, tools
└── transform/        # 协议转换
    ├── mod.rs        # 模块导出
    ├── request.rs    # Anthropic → OpenAI 请求转换（tools, images, tool_results）
    ├── response.rs   # OpenAI → Anthropic 响应转换，错误映射
    ├── stream.rs     # SSE 流式转换（状态机，OpenAI chunk → Anthropic 事件，tool 流式）
    └── headers.rs    # 请求头转发逻辑（跳过头，app-key/app-sign 注入）
```

## 许可证

MIT
