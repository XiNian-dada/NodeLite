import { createAlertSettingsPanel } from "/assets/index-alert-settings.js";

export function createAdminPanels(deps) {
  const {
    t,
    escapeHtml,
    fmtDurationSeconds,
    fmtDateTime,
    fetchJson,
    postSettingsJson,
    compareVersions,
    generatePassword,
    updateConsole,
  } = deps;

  let latestSettings = null;
  const alertSettings = createAlertSettingsPanel(deps);
  let pendingTwoFactorSetup = null;
  function settingsRoot() {
    return document.getElementById("settings-root");
  }
  function accountRoot() {
    return document.getElementById("account-root");
  }
  async function loadSystemSettings() {
    const root = settingsRoot();
    if (!root) return;
    root.innerHTML = `<div class="empty">${escapeHtml(t("settings.loading"))}</div>`;
    try {
      latestSettings = await fetchJson("/api/settings");
      renderSystemSettings();
    } catch (error) {
      root.innerHTML = `<div class="empty">${escapeHtml(t("settings.load_failed", { error: error.message }))}</div>`;
    }
  }

  async function loadAccountSettings() {
    const root = accountRoot();
    if (!root) return;
    root.innerHTML = `<div class="empty">${escapeHtml(t("settings.loading"))}</div>`;
    try {
      latestSettings = await fetchJson("/api/settings");
      renderAccountSettings();
    } catch (error) {
      root.innerHTML = `<div class="empty">${escapeHtml(t("settings.load_failed", { error: error.message }))}</div>`;
    }
  }

  function applyChrome(activeTab) {
    if (activeTab === "settings" && latestSettings) {
      renderSystemSettings();
    }
    if (activeTab === "account" && latestSettings) {
      renderAccountSettings();
    }
    alertSettings.applyChrome(activeTab);
  }

  function kv(label, value) {
    return `<div><span>${escapeHtml(label)}</span><span>${escapeHtml(value ?? t("common.not_available"))}</span></div>`;
  }

  function tokenTable(agents) {
    if (!agents.length) return `<div class="empty">${escapeHtml(t("settings.tokens.empty"))}</div>`;
    const rows = agents.map((agent) => {
      const seconds = agent.token_expires_in_secs;
      const cls = seconds == null ? "" : seconds <= 0 ? "token-expired" : seconds < 7 * 86400 ? "token-expiring" : "token-ok";
      return `<tr>
        <td>${escapeHtml(agent.node_label || agent.node_id)}<div class="settings-note">${escapeHtml(agent.node_id)}</div></td>
        <td>${escapeHtml(agent.online ? t("common.online") : t("common.offline"))}</td>
        <td>${escapeHtml(agent.agent_version || t("common.not_available"))}</td>
        <td>${escapeHtml(agent.remote_ip || t("common.not_available"))}</td>
        <td>${escapeHtml(agent.token_expires_at ? fmtDateTime(agent.token_expires_at) : t("settings.token.no_expiry"))}</td>
        <td class="numeric ${cls}">${escapeHtml(fmtDurationSeconds(seconds))}</td>
      </tr>`;
    }).join("");
    return `<table class="token-table">
      <thead><tr>
        <th>${escapeHtml(t("settings.tokens.node"))}</th>
        <th>${escapeHtml(t("settings.tokens.status"))}</th>
        <th>${escapeHtml(t("settings.tokens.agent"))}</th>
        <th>${escapeHtml(t("settings.tokens.ip"))}</th>
        <th>${escapeHtml(t("settings.tokens.expires_at"))}</th>
        <th>${escapeHtml(t("settings.tokens.remaining"))}</th>
      </tr></thead>
      <tbody>${rows}</tbody>
    </table>`;
  }

  function renderSystemSettings() {
    const settings = latestSettings;
    const root = settingsRoot();
    if (!settings || !root) return;
    const auth = settings.auth || {};
    root.innerHTML = `
      <article class="settings-card settings-version-card">
        <h2>${escapeHtml(t("settings.version.title"))}</h2>
        <div class="settings-kv">
          ${kv(t("settings.version.current"), settings.server_version)}
          ${kv(t("settings.version.repository"), settings.repository)}
          ${kv(t("settings.version.public_url"), settings.public_base_url)}
          ${kv(t("settings.version.listen"), settings.listen)}
        </div>
        <div class="settings-actions">
          <button type="button" class="settings-button primary" id="settings-check-update">${escapeHtml(t("settings.version.check_updates"))}</button>
          <button type="button" class="settings-button" id="settings-view-update-log">${escapeHtml(t("settings.version.view_update_log"))}</button>
          <a class="settings-button" href="${escapeHtml(settings.updates.latest_release_url)}" target="_blank" rel="noreferrer">${escapeHtml(t("settings.version.open_release"))}</a>
        </div>
        <form class="settings-form manual-update-form" id="server-update-form">
          <p class="settings-note">${escapeHtml(t(auth.two_factor_enabled ? "settings.version.manual_update_note_2fa" : "settings.version.manual_update_note_password"))}</p>
          ${serverUpdateConfirmationField(auth)}
          <div class="settings-actions">
            <button type="submit" class="settings-button primary">${escapeHtml(t("settings.version.update_now"))}</button>
          </div>
        </form>
        <div id="settings-update-message" class="settings-message"></div>
      </article>
      <article class="settings-card settings-card-wide settings-ops-card">
        <h2>${escapeHtml(t("settings.ops.title"))}</h2>
        <div class="settings-kv">
          ${kv(t("settings.ops.config"), settings.config_path)}
          ${kv(t("settings.ops.registry"), settings.registry_path)}
          ${kv(t("settings.ops.history"), settings.history_db_path)}
          ${kv(t("settings.ops.snapshot"), settings.snapshot_path)}
        </div>
        <p class="settings-note">${escapeHtml(t("settings.ops.server_upgrade"))}</p>
        <pre class="settings-note settings-code">${escapeHtml(settings.updates.server_upgrade_command)}</pre>
        <p class="settings-note">${escapeHtml(t("settings.ops.agent_upgrade"))}</p>
        <pre class="settings-note settings-code">${escapeHtml(settings.updates.agent_upgrade_command)}</pre>
      </article>
      <article class="settings-card settings-card-wide settings-token-card">
        <h2>${escapeHtml(t("settings.tokens.title"))}</h2>
        ${tokenTable(settings.agents || [])}
      </article>
    `;
    bindSystemSettingsActions(settings);
  }

  function renderAccountSettings() {
    const settings = latestSettings;
    const root = accountRoot();
    if (!settings || !root) return;
    const auth = settings.auth || {};
    root.innerHTML = `
      <article class="settings-card">
        <h2>${escapeHtml(t("settings.security.title"))}</h2>
        <div class="settings-kv">
          ${kv(t("settings.security.auth"), auth.enabled ? t("common.online") : t("common.offline"))}
          ${kv(t("settings.security.username"), auth.username || t("common.not_available"))}
          ${kv(t("settings.security.2fa"), auth.two_factor_enabled ? t("settings.enabled") : t("settings.disabled"))}
          ${kv(t("settings.security.session_ttl"), fmtDurationSeconds(auth.session_ttl_secs))}
        </div>
        <p class="settings-note">${escapeHtml(t("settings.security.2fa_note"))}</p>
        ${twoFactorControls(auth)}
        <div class="settings-actions">
          <button type="button" class="settings-button danger" id="account-logout">${escapeHtml(t("settings.security.logout"))}</button>
        </div>
      </article>

      <article class="settings-card">
        <h2>${escapeHtml(t("settings.password.title"))}</h2>
        <form class="settings-form" id="password-form">
          <label>${escapeHtml(t("settings.password.current"))}<input class="settings-input" type="password" name="current_password" autocomplete="current-password" required></label>
          <label>${escapeHtml(t("settings.password.new"))}<input class="settings-input" type="password" name="new_password" autocomplete="new-password" minlength="8" required></label>
          <div class="settings-actions">
            <button type="button" class="settings-button" id="password-generate">${escapeHtml(t("settings.password.generate"))}</button>
            <button type="submit" class="settings-button primary">${escapeHtml(t("settings.password.submit"))}</button>
          </div>
          <div id="password-message" class="settings-message"></div>
        </form>
      </article>
    `;
    bindAccountActions(settings);
  }

  function serverUpdateConfirmationField(auth) {
    if (auth.two_factor_enabled) {
      return `<label>${escapeHtml(t("settings.security.verification_code"))}<input class="settings-input" type="text" name="code" inputmode="numeric" pattern="[0-9]{6}" maxlength="6" autocomplete="one-time-code" required></label>`;
    }
    return `<label>${escapeHtml(t("settings.password.current"))}<input class="settings-input" type="password" name="current_password" autocomplete="current-password" required></label>`;
  }

  function twoFactorControls(auth) {
    if (auth.two_factor_enabled) {
      return `<form class="settings-form totp-setup" id="totp-disable-form">
        <p class="settings-note">${escapeHtml(t("settings.security.disable_note"))}</p>
        <label>${escapeHtml(t("settings.password.current"))}<input class="settings-input" type="password" name="current_password" autocomplete="current-password" required></label>
        <label>${escapeHtml(t("settings.security.verification_code"))}<input class="settings-input" type="text" name="code" inputmode="numeric" pattern="[0-9]{6}" maxlength="6" autocomplete="one-time-code" required></label>
        <div class="settings-actions">
          <button type="submit" class="settings-button danger">${escapeHtml(t("settings.security.disable_2fa"))}</button>
        </div>
        <div id="totp-message" class="settings-message"></div>
      </form>`;
    }
    if (!pendingTwoFactorSetup) {
      return `<div class="settings-actions">
        <button type="button" class="settings-button primary" id="totp-start">${escapeHtml(t("settings.security.start_2fa"))}</button>
      </div>
      <div id="totp-message" class="settings-message"></div>`;
    }
    return `<div class="totp-setup" id="totp-setup-panel">
      <div class="totp-setup-grid">
        <div><div class="totp-qr-wrap">${pendingTwoFactorSetup.qr_svg}</div></div>
        <div>
          <h3>${escapeHtml(t("settings.security.scan_qr"))}</h3>
          <p class="settings-note">${escapeHtml(t("settings.security.setup_note"))}</p>
          <div class="secret-box">${escapeHtml(pendingTwoFactorSetup.secret)}</div>
        </div>
      </div>
      <form class="settings-form" id="totp-enable-form">
        <label>${escapeHtml(t("settings.password.current"))}<input class="settings-input" type="password" name="current_password" autocomplete="current-password" required></label>
        <label>${escapeHtml(t("settings.security.verification_code"))}<input class="settings-input" type="text" name="code" inputmode="numeric" pattern="[0-9]{6}" maxlength="6" autocomplete="one-time-code" required></label>
        <div class="settings-actions">
          <button type="submit" class="settings-button primary">${escapeHtml(t("settings.security.enable_2fa"))}</button>
          <button type="button" class="settings-button" id="totp-cancel">${escapeHtml(t("settings.security.cancel_setup"))}</button>
        </div>
        <div id="totp-message" class="settings-message"></div>
      </form>
    </div>`;
  }

  function bindSystemSettingsActions(settings) {
    document.getElementById("settings-check-update")?.addEventListener("click", () => checkForUpdates(settings));
    document.getElementById("settings-view-update-log")?.addEventListener("click", () => {
      updateConsole.open();
      void updateConsole.fetch({ reset: true });
    });
    document.getElementById("server-update-form")?.addEventListener("submit", submitServerUpdate);
  }

  function bindAccountActions(settings) {
    document.getElementById("account-logout")?.addEventListener("click", () => {
      try { window.localStorage.removeItem("nodelite.auth.timestamp"); } catch (_e) {}
      window.location.href = "/logout-and-reauth";
    });
    document.getElementById("totp-start")?.addEventListener("click", startTwoFactorSetup);
    document.getElementById("totp-cancel")?.addEventListener("click", () => {
      pendingTwoFactorSetup = null;
      renderAccountSettings();
    });
    document.getElementById("totp-enable-form")?.addEventListener("submit", submitTwoFactorEnable);
    document.getElementById("totp-disable-form")?.addEventListener("submit", submitTwoFactorDisable);
    document.getElementById("password-generate")?.addEventListener("click", () => {
      const input = document.querySelector("#password-form [name=new_password]");
      if (input) input.value = generatePassword();
    });
    document.getElementById("password-form")?.addEventListener("submit", submitPasswordChange);
  }

  async function checkForUpdates(settings) {
    const message = document.getElementById("settings-update-message");
    if (!message) return;
    message.className = "settings-message";
    message.textContent = t("settings.version.checking");
    try {
      const api = String(settings.repository || "").replace("https://github.com/", "https://api.github.com/repos/") + "/releases/latest";
      const release = await fetchJson(api);
      const latest = String(release.tag_name || "").replace(/^v/, "");
      const current = String(settings.server_version || "").replace(/^v/, "");
      const newer = compareVersions(latest, current) > 0;
      message.className = `settings-message ${newer ? "ok" : ""}`;
      message.textContent = newer
        ? t("settings.version.update_available", { version: latest })
        : t("settings.version.up_to_date", { version: current });
    } catch (error) {
      message.className = "settings-message error";
      message.textContent = t("settings.version.check_failed", { error: error.message });
    }
  }

  async function submitServerUpdate(event) {
    event.preventDefault();
    const form = event.currentTarget;
    const message = document.getElementById("settings-update-message");
    const auth = latestSettings?.auth || {};
    const payload = auth.two_factor_enabled
      ? { code: form.code.value }
      : { current_password: form.current_password.value };
    message.className = "settings-message";
    message.textContent = t("settings.version.update_starting");
    updateConsole.open();
    updateConsole.reset();
    updateConsole.setStatus("waiting", t("settings.version.console_status_waiting"));
    updateConsole.setMeta(t("settings.version.console_preparing"));
    updateConsole.setText(`[client] ${t("settings.version.update_starting")}`);
    try {
      await postSettingsJson("/api/settings/update/server", payload);
      message.className = "settings-message ok";
      message.textContent = t("settings.version.update_started");
      updateConsole.appendLine(`[client] ${t("settings.version.update_started")}`);
      updateConsole.setStatus("running", t("settings.version.console_status_running"));
      updateConsole.setMeta(t("settings.version.console_connecting"));
      void updateConsole.fetch({ reset: true });
    } catch (error) {
      message.className = "settings-message error";
      message.textContent = t("settings.version.update_failed", { error: error.message });
      updateConsole.appendLine(`[client] ${t("settings.version.update_failed", { error: error.message })}`);
      updateConsole.setStatus("error", t("settings.version.console_status_error"));
      updateConsole.setMeta(t("settings.version.console_failed_to_start"));
    }
  }

  async function startTwoFactorSetup() {
    const message = document.getElementById("totp-message");
    if (!message) return;
    message.className = "settings-message";
    message.textContent = t("settings.security.starting_2fa");
    try {
      pendingTwoFactorSetup = await postSettingsJson("/api/settings/2fa/start", {});
      renderAccountSettings();
    } catch (error) {
      message.className = "settings-message error";
      message.textContent = t("settings.security.action_failed", { error: error.message });
    }
  }

  async function submitTwoFactorEnable(event) {
    event.preventDefault();
    const form = event.currentTarget;
    const message = document.getElementById("totp-message");
    message.className = "settings-message";
    message.textContent = t("settings.security.enabling_2fa");
    try {
      await postSettingsJson("/api/settings/2fa/enable", {
        current_password: form.current_password.value,
        secret: pendingTwoFactorSetup?.secret || "",
        code: form.code.value,
      });
      pendingTwoFactorSetup = null;
      message.className = "settings-message ok";
      message.textContent = t("settings.security.enabled_saved");
      window.setTimeout(() => loadAccountSettings(), 600);
    } catch (error) {
      message.className = "settings-message error";
      message.textContent = t("settings.security.action_failed", { error: error.message });
    }
  }

  async function submitTwoFactorDisable(event) {
    event.preventDefault();
    const form = event.currentTarget;
    const message = document.getElementById("totp-message");
    message.className = "settings-message";
    message.textContent = t("settings.security.disabling_2fa");
    try {
      await postSettingsJson("/api/settings/2fa/disable", {
        current_password: form.current_password.value,
        code: form.code.value,
      });
      message.className = "settings-message ok";
      message.textContent = t("settings.security.disabled_saved");
      window.setTimeout(() => loadAccountSettings(), 600);
    } catch (error) {
      message.className = "settings-message error";
      message.textContent = t("settings.security.action_failed", { error: error.message });
    }
  }

  async function submitPasswordChange(event) {
    event.preventDefault();
    const form = event.currentTarget;
    const message = document.getElementById("password-message");
    message.className = "settings-message";
    message.textContent = t("settings.password.saving");
    try {
      await postSettingsJson("/api/settings/password", {
        current_password: form.current_password.value,
        new_password: form.new_password.value,
      });
      message.className = "settings-message ok";
      message.textContent = t("settings.password.saved");
      try { window.localStorage.removeItem("nodelite.auth.timestamp"); } catch (_e) {}
      window.setTimeout(() => { window.location.href = "/logout-and-reauth"; }, 900);
    } catch (error) {
      message.className = "settings-message error";
      message.textContent = t("settings.password.failed", { error: error.message });
    }
  }

  return {
    applyChrome,
    loadAccountSettings,
    loadAlertSettings: alertSettings.loadAlertSettings,
    loadSystemSettings,
  };
}
