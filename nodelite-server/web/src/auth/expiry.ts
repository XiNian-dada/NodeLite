export const AUTH_TIMESTAMP_KEY = 'nodelite.auth.timestamp';
export const AUTH_EXPIRY_MS = 24 * 60 * 60 * 1000;
export const LOGOUT_PATH = '/logout-and-reauth';

/**
 * Sliding 24-hour client-side auth window. Mirrors the legacy IIFE at
 * assets/index.html:24-44 / node.html:29-49.
 *
 * - First load (no timestamp): write current time, continue.
 * - Within 24h: refresh timestamp, continue.
 * - >24h: clear the key and redirect to /logout-and-reauth so the server
 *   sends WWW-Authenticate and the browser flushes Basic Auth credentials.
 *
 * The timestamp is initially set on TOTP success at verify-2fa.html:388.
 *
 * Returns `true` if the page should continue booting, `false` if a redirect
 * has been initiated (caller should bail out).
 */
export function checkAuthExpiry(): boolean {
  try {
    const raw = window.localStorage.getItem(AUTH_TIMESTAMP_KEY);
    const now = Date.now();
    if (raw !== null) {
      const ts = Number.parseInt(raw, 10);
      if (Number.isFinite(ts) && now - ts > AUTH_EXPIRY_MS) {
        window.localStorage.removeItem(AUTH_TIMESTAMP_KEY);
        window.location.assign(LOGOUT_PATH);
        return false;
      }
    }
    window.localStorage.setItem(AUTH_TIMESTAMP_KEY, now.toString());
    return true;
  } catch {
    // localStorage unavailable — fail open (don't lock the user out of a
    // working session because of private-browsing mode).
    return true;
  }
}
