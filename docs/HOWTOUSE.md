# api-switch 使用说明

## 简介

基于 Rust 的高性能 HTTP 网关，将 [Anthropic Messages API](https://docs.anthropic.com/en/api/messages) 和 [OpenAI Chat Completions API](https://platform.openai.com/docs/api-reference/chat) 格式，转发至**多后端 OpenAI 兼容**的服务。支持流式（SSE）和非流式两种模式，全面支持接入各大主流 AI Coding Agent。

> 只支持后端认证采用 **AppKey** 和 **AppSign** 鉴权，而**非标准 Authorization 鉴权**。每个模型独立配置认证凭据和后端地址，请求通过 `model` 字段自动路由。

## 交付文件

| 文件 | 说明 |
|------|------|
| `api-switch.exe` | 网关主程序 |
| `config.yaml.example` | 配置文件模板 |

## 快速开始

### 1. 创建配置文件

将 `config.yaml.example` 复制为 `config.yaml`，并修改为实际的参数：

```yaml
models:
  - name: qwen35-397b
    app_key: "你的AppKey"
    app_sign: "你的AppSign"
    base_url: http://你的后端地址:端口/
  - name: glm-5
    app_key: "key2"
    app_sign: "sign2"
    base_url: http://另一个后端地址:端口/
port: 3000
```

**配置文件必须和 `api-switch.exe` 放在同一个目录下。** 或通过 `CONFIG_PATH` 环境变量指定路径。

### 2. 启动服务

**方式一：双击运行**

直接双击 `api-switch.exe`，程序将读取同目录下的 `config.yaml` 并启动服务。

> 如果弹出防火墙提示，请点击允许！

**方式二：命令行运行**

打开命令提示符（cmd）或 PowerShell，进入 exe 所在目录，执行：

```cmd
api-switch.exe
```

### 3. 验证服务

启动成功后，终端会显示：

```
Gateway listening on 0.0.0.0:3000
```

使用以下命令测试：

```bash
# 健康检查
curl http://localhost:3000/health
```

### 4. AI Coding Agent 配置

#### Claude Code

推荐通过**CC-Switch**配置，供应商选择自定义供应商

| 请求地址       | `http://localhost:{port}` （根据配置文件填写） |
| -------------- | ------------------------------------------ |
| API key        | 随便填                                     |
| 高级选项中模型 | 填写 `config.yaml` 中 `models` 里定义的 `name`，例如：`qwen35-397b` |

参考配置`JSON`，也可自己更新`env`

```json
{
  "env": {
    "ANTHROPIC_DEFAULT_OPUS_MODEL": "glm-5",
    "ANTHROPIC_DEFAULT_SONNET_MODEL": "glm-5",
    "ANTHROPIC_AUTH_TOKEN": "12345",
    "ANTHROPIC_BASE_URL": "http://localhost:3000",
    "ANTHROPIC_MODEL": "glm-5",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "glm-5",
    "API_TIMEOUT_MS": "3000000",
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": 1
  },
  "includeCoAuthoredBy": false
}
```

#### OpenCode

编辑`C:\Users\user\.config\opencode\opencode.jsonc`文件

> user要更换为自己的用户名

参考配置如下：
```json
{
  "$schema": "https://opencode.ai/config.json",
  "provider": {
    "JT-EDA": {
      "npm": "@ai-sdk/openai-compatible",
      "name": "my-model",
      "options": {
        "baseURL": "http://localhost:3000/v1"
      },
      "models": {
        "glm-5": {
          "name": "glm-5"
        },
        "qwen35-397b": {	// qwen35-397b才是模型真正请求的model！！name只做展示
          "name": "Qwen3.5-397b"
        }
      }
    }
  }
}
```

#### TRAE

在`设置 -> 模型`中点击`添加模型`

| API格式        | OpenAI Chat Completions格式 |
| -------------- | --------------------------- |
| 自定义请求地址 | http://localhost:3000/v1    |
| 模型ID         | 根据配置文件填写            |
| API密钥        | 随便填，例如：12345         |

## 配置说明

### models（必填）

模型列表，每个模型包含以下字段：

| 字段 | 必填 | 说明 |
|------|------|------|
| `name` | 是 | 模型名称，客户端请求中的 `model` 字段需与此匹配 |
| `app_key` | 是 | 后端服务要求的 AppKey |
| `app_sign` | 是 | 后端服务要求的 AppSign |
| `base_url` | 是 | OpenAI 兼容后端的基础地址，**不要带 `/v1/chat/completions`** |

### port（可选）

| 字段 | 必填 | 默认值 | 说明 |
|------|------|--------|------|
| `port` | 否 | `3000` | 网关监听的端口号 |

### 关于 base_url

填写后端服务的基础地址即可，程序会自动拼接 `/v1/chat/completions`。例如：

```yaml
# 正确 ✓
base_url: https://api.openai.com/v1/

# 错误 ✗ — 不要带 /v1/chat/completions
base_url: https://api.openai.com/v1/chat/completions
```

## API 端点

| 方法 | 路径 | 说明 |
|------|------|------|
| `POST` | `/v1/messages` | Anthropic 协议转换入口 |
| `POST` | `/v1/chat/completions` | OpenAI 透传代理（无协议转换） |
| `GET` | `/health` | 健康检查，转发到第一个模型后端的 `/health` |

## 工作流程

```
客户端（Anthropic 格式）
    │
    ▼
POST /v1/messages  →  api-switch 网关
    │
    ├─ 根据 model 字段查找对应模型配置
    ├─ 解析 Anthropic 请求
    ├─ 转换为 OpenAI 格式
    ├─ 添加 App-Key、App-Sign 请求头
    ├─ 转发到对应后端 /v1/chat/completions
    ├─ 将 OpenAI 响应转换为 Anthropic 格式
    └─ 返回客户端
```

## 常见问题

**Q: 启动时提示 "Failed to read config file"**

A: 确保 `config.yaml` 与 `api-switch.exe` 在同一目录下。或者设置 `CONFIG_PATH` 环境变量指向正确的文件路径。

**Q: 如何修改监听端口？**

A: 在 `config.yaml` 中修改 `port` 字段，例如 `port: 8080`。该字段可选，默认为 3000。

**Q: 需要重启生效配置修改吗？**

A: 是的，修改 `config.yaml` 后需要重启 `api-switch.exe`。

**Q: 支持哪些 Anthropic 参数？**

A: 支持 `model`、`messages`（文本/图片）、`system`、`max_tokens`、`temperature`、`top_p`、`stop_sequences`、`stream`。`top_k` 会被忽略（OpenAI 无对应参数）。

**Q: 请求中 model 字段填什么？**

A: 填写 `config.yaml` 中 `models` 列表里某个模型的 `name` 值。未知模型名称将返回 400 错误。
