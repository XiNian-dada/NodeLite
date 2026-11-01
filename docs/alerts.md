---
title: 告警与巡检
nav_order: 5
---

# 告警与巡检

告警可以在面板的告警页配置，也可以写在 `server.toml` 的 `[alerts]` 段。面板保存时会校验取值范围，不合法的值返回错误而不会写入。

## 渠道

```toml
[alerts]
enabled = true

[alerts.smtp]
enabled = true
host = "smtp.example.com"
port = 587
transport = "start_tls"
username = "alert@example.com"
password = "your-smtp-password"
sender = "alert@example.com"
recipients = ["admin@example.com"]
send_resolved = true

[alerts.webhook]
enabled = true
url = "https://hooks.example.com/nodelite"
# secret = "..."   # 设置后请求带 X-NodeLite-Signature: sha256=<HMAC>
send_resolved = true
```

- **SMTP**：支持 STARTTLS。
- **Webhook**：向 `url` POST 一个 JSON 对象（包含事件类型、规则、节点和读数）。接收方需要能处理这个格式；Telegram、Slack 等服务通常需要一个中间转换层。企业微信群机器人地址（`qyapi.weixin.qq.com/cgi-bin/webhook/send`）会自动使用企业微信的文本消息格式，并检查返回的 `errcode`。
- 每个渠道独立记录投递结果：一个渠道失败时只重试这个渠道，其他渠道不会重复收到；告警恢复时，恢复通知发给收到过触发通知的渠道。

## 规则

每条规则选择一个指标、比较方式和阈值：

| 指标 | 含义 |
| --- | --- |
| `cpu_usage_percent` | CPU 使用率（%） |
| `memory_usage_percent` | 内存使用率（%） |
| `disk_usage_percent` | 磁盘使用率（%） |
| `latency_ms` | Server 测得的心跳延迟（毫秒） |
| `offline_minutes` | 节点离线时长（分钟） |
| `traffic_usage_percent` | 本月套餐流量使用率（%），可以超过 100 |

```toml
[[alerts.rules]]
id = "cpu-avg-hot"            # 稳定且唯一
name = "CPU 5 分钟平均超过 85%"
enabled = true
metric = "cpu_usage_percent"
comparator = "gt"
threshold = 85
window_minutes = 5            # 1..=20160
cooldown_minutes = 30         # 1..=43200，告警持续时重复提醒的间隔
severity = "warning"
scope_mode = "all"            # 或 "node_ids" / "tags"，配合下面两项限定范围
node_ids = []
tags = []
delivery = ["smtp", "webhook"]
send_resolved = true
```

CPU、内存、磁盘的阈值不超过 100。旧配置中超出范围的值会在启动时截断到上限并给出警告。

## 每日巡检

每天在 `local_time` 汇总过去一段时间（`lookback_hours`，最多 336 小时）内的离线、高延迟和高负载节点，发送到选定的渠道：

```toml
[alerts.inspection]
enabled = true
local_time = "09:00"
lookback_hours = 24
delivery = ["smtp", "webhook"]
offline_grace_minutes = 10
latency_warn_ms = 250
cpu_warn_percent = 85
memory_warn_percent = 90
```
