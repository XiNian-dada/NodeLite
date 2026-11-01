---
title: 反向代理
nav_order: 3
---

# 反向代理

Server 默认只监听 `127.0.0.1:8080`。生产环境由 Nginx 或 Caddy 终结 TLS，再转发给 Server。代理需要：

- 转发 WebSocket 升级（`/ws` 用于 Agent，`/ws/browser` 用于面板实时推送）；
- 保留浏览器访问时的原始 `Host`；
- 把代理的地址写进 `server.toml` 的 `trusted_proxies`，否则所有 Agent 会被识别成同一个代理 IP，触发 `/ws` 限流。

## Nginx

保存为 `/etc/nginx/conf.d/nodelite.conf`，替换域名和证书路径：

```nginx
server {
    listen 443 ssl http2;
    server_name monitor.example.com;

    ssl_certificate     /etc/letsencrypt/live/monitor.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/monitor.example.com/privkey.pem;

    # 面板与 API
    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    # Agent 与浏览器的 WebSocket 连接
    location /ws {
        proxy_pass http://127.0.0.1:8080/ws;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_read_timeout 120s;
        proxy_send_timeout 120s;
    }

    # Agent 安装脚本
    location /install/ {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

如果代理监听的不是 443 等默认端口（例如 `listen 8443 ssl`），把 `proxy_set_header Host $host;` 改为 `proxy_set_header Host $http_host;`：`$host` 不含端口，会导致面板实时推送的来源校验失败。

## Caddy

Caddy 默认会转发 WebSocket、保留 `Host` 并设置 `X-Forwarded-*` 头，最小配置即可：

```caddy
monitor.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

## Server 侧配置

```toml
[server]
public_base_url = "https://monitor.example.com"
# 代理所在的地址或网段；代理与 Server 同机时为 127.0.0.1/32
trusted_proxies = ["127.0.0.1/32"]
```

`public_base_url` 只能写来源（协议、主机和端口），不能带路径；Server 用它生成 WebSocket 地址和安装命令。如果前面还有 CDN 或 WAF，把它们的出口网段也加入 `trusted_proxies`。

## 浏览器来源校验

面板的实时推送（`/ws/browser`）会检查浏览器的 `Origin`，它必须与请求的 `Host` 或 `public_base_url` 同源；`public_base_url` 为 https 时，同主机的 http 页面会被拒绝。代理改写了 `Host` 时，请通过 `public_base_url` 的地址访问面板。被拒绝的握手会在 Server 日志里记录 `Origin` 和 `Host`。
