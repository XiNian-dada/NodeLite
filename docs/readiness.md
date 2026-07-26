# 健康检查与就绪探针

NodeLite Server 提供两个无需认证的探针端点：

- `/healthz`：进程存活检查。只要 HTTP 服务还能响应就返回 `200 OK`。
- `/readyz`：流量就绪检查和结构化运行诊断。HTTP 状态码与 JSON 的 `ready` 字段表示能否承载流量，`status` 和 `problems` 表示更广泛的运行健康度。

## `/readyz` 语义

`/readyz` 将暴露内容分为三类：

| 类别 | JSON 位置或条件 | 当前判定语义 |
| --- | --- | --- |
| 硬就绪检查 | `checks.history_available`、`checks.registry_reload_healthy` | 任一失败时加入对应的 `problems`，返回 `status: "degraded"`、`ready: false` 和 `503 Service Unavailable` |
| 降级诊断条件 | `signals.audit_enabled && !signals.audit_available`；三个写入计数器中的任一个大于 `0` | 分别加入 `audit_unavailable`、`history_dropped_writes`、`audit_dropped_writes` 或 `audit_write_failures`，并返回 `status: "degraded"`；这些条件本身不参与 HTTP 状态码和 `ready` 的判定 |
| 原始观测字段 | `signals` 中 history/audit 队列深度与容量、Agent/Browser WebSocket 当前连接数、总容量及单 IP 限制 | 只报告当前值；服务端目前不为这些字段设置阈值，因此它们本身不会加入 `problems`，也不会改变 `status`、`ready` 或 HTTP 状态码 |

审计日志是安全诊断能力，但不是 Server 接收 Agent、查询节点状态所需的硬依赖。因此，仅审计写入器不可用时，响应组合是：

```json
{
  "status": "degraded",
  "ready": true,
  "problems": ["audit_unavailable"]
}
```

对应的 HTTP 状态码仍为 `200 OK`。如果历史存储或注册表重载检查失败，则响应为 `ready: false`、`status: "degraded"`，HTTP 状态码为 `503 Service Unavailable`。

## 探针与告警配置

- Kubernetes、systemd watchdog 或负载均衡器的就绪判断应使用 `/readyz` 的 HTTP 状态码；需要解析 JSON 时，应读取 `ready`，不要把 `status == "ok"` 当作接流量条件。
- 告警系统应另外监控 `status`、`problems` 和 `signals`。`ready: true` 且 `status: "degraded"` 表示服务仍可接流量，但存在需要运维处理的诊断异常。队列和 WebSocket 容量字段仅提供原始数据，需要由外部监控按部署规模设置阈值。
- 不要仅用 `/healthz` 判断是否应把实例加入流量池；它只验证进程仍能响应。
