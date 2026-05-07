pub fn index_html(refresh_interval_secs: u64) -> String {
    INDEX_TEMPLATE.replace(
        "__REFRESH_MS__",
        &(refresh_interval_secs * 1000).to_string(),
    )
}

pub fn node_html(node_id: &str, refresh_interval_secs: u64) -> String {
    NODE_TEMPLATE
        .replace(
            "__REFRESH_MS__",
            &(refresh_interval_secs * 1000).to_string(),
        )
        .replace(
            "__NODE_ID_JSON__",
            &serde_json::to_string(node_id).unwrap_or_else(|_| "\"\"".to_string()),
        )
}

const INDEX_TEMPLATE: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>XiMonitor</title>
    <style>
      :root {
        color-scheme: light;
        --bg-a: #f2ede2;
        --bg-b: #eef2f4;
        --ink: #18212c;
        --muted: #55616f;
        --line: rgba(24, 33, 44, 0.08);
        --panel: rgba(255, 255, 255, 0.84);
        --good: #1d6a43;
        --bad: #b04736;
        --accent: #0e7490;
        font-family: "Avenir Next", "Segoe UI", sans-serif;
      }
      * { box-sizing: border-box; }
      body {
        margin: 0;
        min-height: 100vh;
        color: var(--ink);
        background:
          radial-gradient(circle at top left, rgba(205, 226, 236, 0.9), transparent 35%),
          radial-gradient(circle at top right, rgba(244, 221, 196, 0.65), transparent 28%),
          linear-gradient(135deg, var(--bg-a), var(--bg-b));
      }
      .shell {
        width: min(1320px, calc(100vw - 32px));
        margin: 0 auto;
        padding: 28px 0 48px;
      }
      .hero {
        display: flex;
        justify-content: space-between;
        gap: 20px;
        align-items: end;
        margin-bottom: 24px;
      }
      .hero h1 {
        margin: 0;
        font: 700 clamp(2.7rem, 5vw, 4.8rem) / 0.9 "Iowan Old Style", "Palatino Linotype", serif;
        letter-spacing: -0.06em;
      }
      .hero p {
        margin: 14px 0 0;
        max-width: 760px;
        color: var(--muted);
        font-size: 1.03rem;
        line-height: 1.7;
      }
      .stamp {
        text-align: right;
        color: var(--muted);
        font-size: 0.92rem;
      }
      .cards {
        display: grid;
        grid-template-columns: repeat(4, minmax(0, 1fr));
        gap: 16px;
        margin-bottom: 22px;
      }
      .card, .node-card {
        background: var(--panel);
        border: 1px solid var(--line);
        border-radius: 22px;
        box-shadow: 0 18px 60px rgba(24, 33, 44, 0.08);
        backdrop-filter: blur(18px);
      }
      .card {
        padding: 18px 20px;
      }
      .card .label {
        color: var(--muted);
        font-size: 0.9rem;
        text-transform: uppercase;
        letter-spacing: 0.08em;
      }
      .card .value {
        margin-top: 10px;
        font-size: clamp(1.8rem, 3vw, 2.5rem);
        font-weight: 700;
        letter-spacing: -0.05em;
      }
      .node-grid {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
        gap: 16px;
      }
      .node-card {
        display: block;
        padding: 18px 18px 16px;
        color: inherit;
        text-decoration: none;
        transition: transform 180ms ease, box-shadow 180ms ease;
      }
      .node-card:hover {
        transform: translateY(-3px);
        box-shadow: 0 24px 70px rgba(24, 33, 44, 0.12);
      }
      .node-head {
        display: flex;
        justify-content: space-between;
        gap: 12px;
        align-items: start;
      }
      .node-title {
        margin: 0;
        font-size: 1.25rem;
      }
      .node-id {
        color: var(--muted);
        font-size: 0.92rem;
        margin-top: 4px;
      }
      .badge {
        border-radius: 999px;
        padding: 6px 10px;
        font-size: 0.78rem;
        font-weight: 700;
        text-transform: uppercase;
        letter-spacing: 0.08em;
      }
      .online { background: rgba(29, 106, 67, 0.12); color: var(--good); }
      .offline { background: rgba(176, 71, 54, 0.12); color: var(--bad); }
      .kv {
        display: grid;
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: 12px 16px;
        margin-top: 16px;
      }
      .kv strong {
        display: block;
        font-size: 1.05rem;
      }
      .kv span {
        color: var(--muted);
        font-size: 0.84rem;
      }
      .empty {
        padding: 26px;
        background: var(--panel);
        border: 1px dashed rgba(24, 33, 44, 0.18);
        border-radius: 20px;
        color: var(--muted);
        text-align: center;
      }
      @media (max-width: 980px) {
        .cards { grid-template-columns: repeat(2, minmax(0, 1fr)); }
      }
      @media (max-width: 720px) {
        .shell { width: calc(100vw - 20px); }
        .hero { display: block; }
        .stamp { text-align: left; margin-top: 12px; }
        .cards { grid-template-columns: 1fr; }
      }
    </style>
  </head>
  <body>
    <div class="shell">
      <section class="hero">
        <div>
          <h1>XiMonitor</h1>
          <p>Read-only node telemetry for CPU, load, memory, disks, throughput, and WebSocket RTT. Configuration stays on disk; the web view stays observational.</p>
        </div>
        <div class="stamp">
          <div>Refreshes every <strong id="refresh-secs"></strong></div>
          <div id="updated-at">Waiting for data…</div>
        </div>
      </section>

      <section class="cards" id="overview"></section>
      <section id="nodes"></section>
    </div>

    <script>
      const REFRESH_MS = __REFRESH_MS__;
      document.getElementById("refresh-secs").textContent = `${Math.round(REFRESH_MS / 1000)}s`;

      function escapeHtml(value) {
        return String(value)
          .replaceAll("&", "&amp;")
          .replaceAll("<", "&lt;")
          .replaceAll(">", "&gt;")
          .replaceAll('"', "&quot;")
          .replaceAll("'", "&#39;");
      }

      function fmtBytes(bytes) {
        if (bytes == null) return "n/a";
        const units = ["B", "KB", "MB", "GB", "TB", "PB"];
        let value = Number(bytes);
        let index = 0;
        while (value >= 1024 && index < units.length - 1) {
          value /= 1024;
          index += 1;
        }
        return `${value.toFixed(value >= 100 || index === 0 ? 0 : 1)} ${units[index]}`;
      }

      function fmtRate(bytes) {
        if (bytes == null) return "n/a";
        return `${fmtBytes(bytes)}/s`;
      }

      function fmtPercent(value) {
        if (value == null || Number.isNaN(Number(value))) return "n/a";
        return `${Number(value).toFixed(1)}%`;
      }

      function fmtLatency(value) {
        if (value == null) return "n/a";
        return `${Math.round(value)} ms`;
      }

      function diskSummary(disks) {
        if (!Array.isArray(disks) || disks.length === 0) return "n/a";
        const total = disks.reduce((sum, disk) => sum + (disk.total_bytes || 0), 0);
        const used = disks.reduce((sum, disk) => sum + (disk.used_bytes || 0), 0);
        if (!total) return "n/a";
        return fmtPercent((used / total) * 100);
      }

      function setOverview(data) {
        const cards = [
          ["Nodes", `${data.online_nodes}/${data.total_nodes}`, "online / total"],
          ["Latency", fmtLatency(data.average_latency_ms), "mean WebSocket RTT"],
          ["Traffic", `${fmtBytes(data.total_rx_bytes)} in`, `${fmtBytes(data.total_tx_bytes)} out`],
          ["Realtime", `${fmtRate(data.current_rx_bytes_per_sec)} down`, `${fmtRate(data.current_tx_bytes_per_sec)} up`],
        ];
        document.getElementById("overview").innerHTML = cards.map(([label, value, sub]) => `
          <article class="card">
            <div class="label">${escapeHtml(label)}</div>
            <div class="value">${escapeHtml(value)}</div>
            <div class="label" style="margin-top:8px;">${escapeHtml(sub)}</div>
          </article>
        `).join("");
        document.getElementById("updated-at").textContent = `Updated ${new Date(data.generated_at).toLocaleString()}`;
      }

      function setNodes(nodes) {
        const root = document.getElementById("nodes");
        if (!Array.isArray(nodes) || nodes.length === 0) {
          root.innerHTML = `<div class="empty">No agents connected yet. Once an agent sends <code>hello</code> and <code>metrics</code>, it will appear here.</div>`;
          return;
        }

        root.innerHTML = `<div class="node-grid">${nodes.map((node) => {
          const snapshot = node.snapshot || {};
          const memory = snapshot.memory || {};
          return `
            <a class="node-card" href="/nodes/${encodeURIComponent(node.identity.node_id)}">
              <div class="node-head">
                <div>
                  <h2 class="node-title">${escapeHtml(node.identity.node_label)}</h2>
                  <div class="node-id">${escapeHtml(node.identity.node_id)} · ${escapeHtml(node.identity.hostname || "unknown host")}</div>
                </div>
                <span class="badge ${node.online ? "online" : "offline"}">${node.online ? "online" : "offline"}</span>
              </div>
              <div class="kv">
                <div><strong>${fmtPercent(snapshot.cpu_usage_percent)}</strong><span>CPU</span></div>
                <div><strong>${fmtPercent(memory.total_bytes ? (memory.used_bytes / memory.total_bytes) * 100 : null)}</strong><span>Memory</span></div>
                <div><strong>${fmtRate(snapshot.network?.rx_bytes_per_sec)}</strong><span>Download</span></div>
                <div><strong>${fmtRate(snapshot.network?.tx_bytes_per_sec)}</strong><span>Upload</span></div>
                <div><strong>${fmtLatency(node.latency_ms)}</strong><span>RTT</span></div>
                <div><strong>${diskSummary(snapshot.disks)}</strong><span>Disks</span></div>
              </div>
            </a>
          `;
        }).join("")}</div>`;
      }

      async function fetchJson(url) {
        const response = await fetch(url, { headers: { "accept": "application/json" } });
        if (!response.ok) throw new Error(`${url} -> ${response.status}`);
        return response.json();
      }

      async function refresh() {
        try {
          const [overview, nodes] = await Promise.all([
            fetchJson("/api/overview"),
            fetchJson("/api/nodes"),
          ]);
          setOverview(overview);
          setNodes(nodes);
        } catch (error) {
          document.getElementById("nodes").innerHTML = `<div class="empty">Failed to load dashboard data: ${escapeHtml(error.message)}</div>`;
        } finally {
          window.setTimeout(refresh, REFRESH_MS);
        }
      }

      refresh();
    </script>
  </body>
</html>
"#;

const NODE_TEMPLATE: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>XiMonitor Node</title>
    <style>
      :root {
        color-scheme: light;
        --bg: #f7f2e9;
        --ink: #1a202b;
        --muted: #5d6875;
        --line: rgba(26, 32, 43, 0.1);
        --panel: rgba(255, 255, 255, 0.87);
        --accent: #0f766e;
        --chart-a: #0f766e;
        --chart-b: #b45309;
        --chart-c: #1d4ed8;
        --chart-d: #be185d;
        font-family: "Avenir Next", "Segoe UI", sans-serif;
      }
      * { box-sizing: border-box; }
      body {
        margin: 0;
        min-height: 100vh;
        color: var(--ink);
        background:
          radial-gradient(circle at top left, rgba(208, 228, 227, 0.9), transparent 30%),
          radial-gradient(circle at top right, rgba(250, 228, 195, 0.6), transparent 24%),
          linear-gradient(135deg, var(--bg), #eef1f2);
      }
      .shell {
        width: min(1280px, calc(100vw - 32px));
        margin: 0 auto;
        padding: 24px 0 48px;
      }
      a { color: inherit; }
      .topline {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 18px;
        margin-bottom: 18px;
      }
      .topline .back {
        text-decoration: none;
        color: var(--muted);
        font-weight: 600;
      }
      .hero, .panel {
        background: var(--panel);
        border: 1px solid var(--line);
        border-radius: 24px;
        box-shadow: 0 18px 60px rgba(26, 32, 43, 0.08);
        backdrop-filter: blur(18px);
      }
      .hero {
        padding: 24px;
        margin-bottom: 18px;
      }
      .hero h1 {
        margin: 0;
        font: 700 clamp(2.4rem, 4.8vw, 4.1rem) / 0.92 "Iowan Old Style", "Palatino Linotype", serif;
        letter-spacing: -0.05em;
      }
      .meta {
        margin-top: 10px;
        color: var(--muted);
        line-height: 1.7;
      }
      .stats, .charts {
        display: grid;
        gap: 16px;
      }
      .stats {
        grid-template-columns: repeat(4, minmax(0, 1fr));
        margin-bottom: 18px;
      }
      .panel {
        padding: 18px 20px;
      }
      .label {
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 0.84rem;
      }
      .value {
        margin-top: 8px;
        font-size: clamp(1.5rem, 2.7vw, 2.2rem);
        font-weight: 700;
      }
      .charts {
        grid-template-columns: repeat(2, minmax(0, 1fr));
        margin-bottom: 18px;
      }
      .chart-box {
        height: 210px;
        margin-top: 14px;
        border-radius: 18px;
        background: linear-gradient(180deg, rgba(255,255,255,0.4), rgba(242,245,247,0.85));
        border: 1px solid rgba(26, 32, 43, 0.07);
        display: grid;
        place-items: center;
        overflow: hidden;
        position: relative;
      }
      .disks table {
        width: 100%;
        border-collapse: collapse;
      }
      .disks th, .disks td {
        padding: 12px 0;
        text-align: left;
        border-bottom: 1px solid rgba(26, 32, 43, 0.08);
      }
      .disks th {
        color: var(--muted);
        font-size: 0.83rem;
        text-transform: uppercase;
        letter-spacing: 0.08em;
      }
      .empty {
        color: var(--muted);
      }
      @media (max-width: 960px) {
        .stats, .charts { grid-template-columns: 1fr; }
      }
      @media (max-width: 720px) {
        .shell { width: calc(100vw - 20px); }
        .topline { display: block; }
      }
    </style>
  </head>
  <body>
    <div class="shell">
      <div class="topline">
        <a class="back" href="/">← Back to dashboard</a>
        <div id="updated" class="label">Waiting for node data…</div>
      </div>

      <section class="hero">
        <h1 id="title">Loading node…</h1>
        <div class="meta" id="meta"></div>
      </section>

      <section class="stats" id="stats"></section>

      <section class="charts">
        <article class="panel">
          <div class="label">CPU Usage</div>
          <div class="chart-box" id="chart-cpu"></div>
        </article>
        <article class="panel">
          <div class="label">Memory Usage</div>
          <div class="chart-box" id="chart-memory"></div>
        </article>
        <article class="panel">
          <div class="label">Download / Upload</div>
          <div class="chart-box" id="chart-network"></div>
        </article>
        <article class="panel">
          <div class="label">WebSocket RTT</div>
          <div class="chart-box" id="chart-latency"></div>
        </article>
      </section>

      <section class="panel disks">
        <div class="label">Mounted Disks</div>
        <div id="disks" style="margin-top: 14px;"></div>
      </section>
    </div>

    <script>
      const NODE_ID = __NODE_ID_JSON__;
      const REFRESH_MS = __REFRESH_MS__;

      function escapeHtml(value) {
        return String(value)
          .replaceAll("&", "&amp;")
          .replaceAll("<", "&lt;")
          .replaceAll(">", "&gt;")
          .replaceAll('"', "&quot;")
          .replaceAll("'", "&#39;");
      }

      function fmtBytes(bytes) {
        if (bytes == null) return "n/a";
        const units = ["B", "KB", "MB", "GB", "TB", "PB"];
        let value = Number(bytes);
        let index = 0;
        while (value >= 1024 && index < units.length - 1) {
          value /= 1024;
          index += 1;
        }
        return `${value.toFixed(value >= 100 || index === 0 ? 0 : 1)} ${units[index]}`;
      }

      function fmtRate(bytes) {
        if (bytes == null) return "n/a";
        return `${fmtBytes(bytes)}/s`;
      }

      function fmtPercent(value) {
        if (value == null || Number.isNaN(Number(value))) return "n/a";
        return `${Number(value).toFixed(1)}%`;
      }

      function fmtLatency(value) {
        if (value == null) return "n/a";
        return `${Math.round(value)} ms`;
      }

      function fetchJson(url) {
        return fetch(url, { headers: { "accept": "application/json" } }).then((response) => {
          if (!response.ok) throw new Error(`${url} -> ${response.status}`);
          return response.json();
        });
      }

      function renderSparkline(points, colors, formatter) {
        if (!Array.isArray(points) || points.length === 0) {
          return `<div class="empty">Waiting for enough history samples…</div>`;
        }

        const width = 640;
        const height = 210;
        const padding = 16;
        const allValues = points.flatMap((point) => point.values).filter((value) => value != null);
        if (allValues.length === 0) {
          return `<div class="empty">No numeric history yet.</div>`;
        }
        const min = Math.min(...allValues);
        const max = Math.max(...allValues);
        const span = Math.max(max - min, 1);

        const series = colors.map((color, seriesIndex) => {
          let started = false;
          const path = points.map((point, pointIndex) => {
            const value = point.values[seriesIndex];
            if (value == null) return null;
            const x = padding + ((width - padding * 2) * pointIndex) / Math.max(points.length - 1, 1);
            const y = height - padding - (((value - min) / span) * (height - padding * 2));
            const command = started ? "L" : "M";
            started = true;
            return `${command}${x.toFixed(1)},${y.toFixed(1)}`;
          }).filter(Boolean).join(" ");
          return `<path d="${path}" fill="none" stroke="${color}" stroke-width="3.2" stroke-linecap="round" stroke-linejoin="round" />`;
        }).join("");

        return `
          <svg viewBox="0 0 ${width} ${height}" width="100%" height="100%" preserveAspectRatio="none" aria-hidden="true">
            <rect x="0" y="0" width="${width}" height="${height}" fill="transparent" />
            ${series}
          </svg>
          <div style="position:absolute;left:18px;bottom:16px;font-size:0.82rem;color:#5d6875;">${escapeHtml(formatter(min))} → ${escapeHtml(formatter(max))}</div>
        `;
      }

      function renderStats(node) {
        const snapshot = node.snapshot || {};
        const memory = snapshot.memory || {};
        const cards = [
          ["CPU", fmtPercent(snapshot.cpu_usage_percent)],
          ["Load 1/5/15", snapshot.load ? `${snapshot.load.one.toFixed(2)} / ${snapshot.load.five.toFixed(2)} / ${snapshot.load.fifteen.toFixed(2)}` : "n/a"],
          ["Download / Upload", `${fmtRate(snapshot.network?.rx_bytes_per_sec)} / ${fmtRate(snapshot.network?.tx_bytes_per_sec)}`],
          ["Latency", fmtLatency(node.latency_ms)],
          ["Memory", `${fmtBytes(memory.used_bytes)} / ${fmtBytes(memory.total_bytes)}`],
          ["Swap", `${fmtBytes(memory.swap_used_bytes)} / ${fmtBytes(memory.swap_total_bytes)}`],
          ["Uptime", snapshot.uptime_secs != null ? `${Math.round(snapshot.uptime_secs / 3600)}h` : "n/a"],
          ["Agent", node.identity.agent_version || "n/a"],
        ];
        document.getElementById("stats").innerHTML = cards.map(([label, value]) => `
          <article class="panel">
            <div class="label">${escapeHtml(label)}</div>
            <div class="value">${escapeHtml(value)}</div>
          </article>
        `).join("");
      }

      function renderDisks(node) {
        const disks = node.snapshot?.disks || [];
        const root = document.getElementById("disks");
        if (disks.length === 0) {
          root.innerHTML = `<div class="empty">No disk metrics reported yet.</div>`;
          return;
        }
        root.innerHTML = `
          <table>
            <thead>
              <tr>
                <th>Device</th>
                <th>Mount</th>
                <th>Filesystem</th>
                <th>Usage</th>
                <th>Capacity</th>
              </tr>
            </thead>
            <tbody>
              ${disks.map((disk) => `
                <tr>
                  <td>${escapeHtml(disk.device)}</td>
                  <td>${escapeHtml(disk.mount_point)}</td>
                  <td>${escapeHtml(disk.fs_type)}</td>
                  <td>${fmtPercent(disk.used_percent)}</td>
                  <td>${fmtBytes(disk.used_bytes)} / ${fmtBytes(disk.total_bytes)}</td>
                </tr>
              `).join("")}
            </tbody>
          </table>
        `;
      }

      function renderHistory(history) {
        document.getElementById("chart-cpu").innerHTML = renderSparkline(
          history.map((point) => ({ values: [point.cpu_usage_percent] })),
          ["var(--chart-a)"],
          (value) => `${value.toFixed(1)}%`
        );
        document.getElementById("chart-memory").innerHTML = renderSparkline(
          history.map((point) => ({ values: [point.memory_used_percent] })),
          ["var(--chart-b)"],
          (value) => `${value.toFixed(1)}%`
        );
        document.getElementById("chart-network").innerHTML = renderSparkline(
          history.map((point) => ({ values: [point.rx_bytes_per_sec, point.tx_bytes_per_sec] })),
          ["var(--chart-c)", "var(--chart-a)"],
          (value) => fmtRate(value)
        );
        document.getElementById("chart-latency").innerHTML = renderSparkline(
          history.map((point) => ({ values: [point.latency_ms] })),
          ["var(--chart-d)"],
          (value) => `${Math.round(value)} ms`
        );
      }

      async function refresh() {
        try {
          const [node, history] = await Promise.all([
            fetchJson(`/api/nodes/${encodeURIComponent(NODE_ID)}`),
            fetchJson(`/api/nodes/${encodeURIComponent(NODE_ID)}/history`),
          ]);
          document.getElementById("title").textContent = node.identity.node_label;
          document.getElementById("meta").innerHTML = `
            ${escapeHtml(node.identity.node_id)} · ${escapeHtml(node.identity.hostname || "unknown host")} ·
            ${escapeHtml(node.identity.os || "unknown os")} ·
            ${escapeHtml(node.online ? "online" : "offline")}
          `;
          document.getElementById("updated").textContent = node.last_seen
            ? `Last seen ${new Date(node.last_seen).toLocaleString()}`
            : "No heartbeat yet";
          renderStats(node);
          renderDisks(node);
          renderHistory(history);
        } catch (error) {
          document.getElementById("title").textContent = "Node unavailable";
          document.getElementById("meta").textContent = error.message;
        } finally {
          window.setTimeout(refresh, REFRESH_MS);
        }
      }

      refresh();
    </script>
  </body>
</html>
"#;
