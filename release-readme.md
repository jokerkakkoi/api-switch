# api-switch 使用说明

## 简介

`api-switch` 是一个协议转换网关，它将 **Anthropic Messages API** 格式的请求转换为 **OpenAI Chat Completions API** 格式，转发到 OpenAI 兼容的后端服务，并将响应转换回 Anthropic 格式返回给客户端。支持普通请求和流式（SSE）请求。

## 交付文件

| 文件 | 说明 |
|------|------|
| `api-switch.exe` | 网关主程序 |
| `config.yaml.example` | 配置文件模板 |

## 快速开始

### 1. 创建配置文件

将 `config.yaml.example` 复制为 `config.yaml`，并修改为实际的参数：

```yaml
app_key: "你的AppKey"
app_sign: "你的AppSign"
openai_base_url: http://你的后端地址:端口/路径/ # 不要带/v1/chat/completions
port: 3000
```

**配置文件必须和 `api-switch.exe` 放在同一个目录下。**

### 2. 启动服务

**方式一：双击运行**

直接双击 `api-switch.exe`，程序将读取同目录下的 `config.yaml` 并启动服务。

> 这里弹出来的防火墙要点允许！

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

### 4. cc-switch配置

| 请求地址       | http://localhost:{port} # 根据配置文件填写 |
| -------------- | ------------------------------------------ |
| API key        | 随便填                                     |
| 高级选项中模型 | 全部换成数发对应的模型，例如：qwen35-397b  |

## 配置说明

| 字段 | 必填 | 默认值 | 说明 |
|------|------|--------|------|
| `app_key` | 是 | — | 后端服务要求的 AppKey |
| `app_sign` | 是 | — | 后端服务要求的 AppSign |
| `openai_base_url` | 是 | — | OpenAI 兼容后端的地址，**不要带 `/v1/chat/completions`** |
| `port` | 是 | `3000` | 网关监听的端口号 |

### 关于 openai_base_url

填写后端服务的基础地址即可，程序会自动拼接 `/v1/chat/completions`。例如：

```yaml
# 正确 ✓
openai_base_url: https://api.openai.com/v1/

# 错误 ✗ — 不要带 /v1/chat/completions
openai_base_url: https://api.openai.com/v1/chat/completions
```

## API 端点

| 方法 | 路径 | 说明 |
|------|------|------|
| `POST` | `/v1/messages` | Anthropic 协议转换入口 |
| `GET` | `/health` | 健康检查，转发到后端的 `/health` |

## 工作流程

```
客户端（Anthropic 格式）
    │
    ▼
POST /v1/messages  →  api-switch 网关
    │
    ├─ 解析 Anthropic 请求
    ├─ 转换为 OpenAI 格式
    ├─ 添加 App-Key、App-Sign 请求头
    ├─ 转发到后端 /v1/chat/completions
    ├─ 将 OpenAI 响应转换为 Anthropic 格式
    └─ 返回客户端
```

## 常见问题

**Q: 启动时提示 "Failed to read config file"**

A: 确保 `config.yaml` 与 `api-switch.exe` 在同一目录下。或者设置 `CONFIG_PATH` 环境变量指向正确的文件路径。

**Q: 如何修改监听端口？**

A: 在 `config.yaml` 中修改 `port` 字段，例如 `port: 8080`。

**Q: 需要重启生效配置修改吗？**

A: 是的，修改 `config.yaml` 后需要重启 `api-switch.exe`。

**Q: 支持哪些 Anthropic 参数？**

A: 支持 `model`、`messages`（文本/图片）、`system`、`max_tokens`、`temperature`、`top_p`、`stop_sequences`、`stream`。`top_k` 会被忽略（OpenAI 无对应参数）。
