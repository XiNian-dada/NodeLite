/**
 * Single API entry point. Implements plan §3.5.3 verbatim:
 * - credentials: same-origin so Basic Auth + cookies flow
 * - redirect: follow + URL endpoint check identifies 2FA redirects
 * - 401 clears the auth timestamp and bounces to /logout-and-reauth
 * - non-JSON 200 is treated as an error (defensive against silent
 *   redirect/error-page HTML being parsed as JSON)
 */

import { AUTH_TIMESTAMP_KEY } from '@/auth/expiry';

export const VERIFY_2FA_PATH = '/verify-2fa';
export const LOGOUT_PATH = '/logout-and-reauth';

export class ApiError extends Error {
  constructor(
    public readonly status: number,
    public readonly body: string,
  ) {
    super(`API error ${status}: ${body}`);
    this.name = 'ApiError';
  }
}

/**
 * Thrown when the client has already initiated a full-page navigation
 * (to /verify-2fa or /logout-and-reauth). Caller should bail out of any
 * subsequent promise chain — the SPA is about to be torn down.
 */
export class ApiAbortError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'ApiAbortError';
  }
}

export async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    ...init,
    credentials: 'same-origin',
    redirect: 'follow',
    headers: { Accept: 'application/json', ...init?.headers },
  });

  if (res.redirected && new URL(res.url).pathname === VERIFY_2FA_PATH) {
    window.location.assign(VERIFY_2FA_PATH);
    throw new ApiAbortError('redirecting to verify-2fa');
  }

  if (res.status === 401) {
    try {
      window.localStorage.removeItem(AUTH_TIMESTAMP_KEY);
    } catch {
      /* localStorage unavailable — fall through to redirect anyway */
    }
    window.location.assign(LOGOUT_PATH);
    throw new ApiAbortError('redirecting to logout-and-reauth');
  }

  if (!res.ok) {
    throw new ApiError(res.status, await res.text());
  }

  const ct = res.headers.get('content-type') ?? '';
  if (!ct.includes('application/json')) {
    throw new ApiError(res.status, `unexpected content-type: ${ct}`);
  }

  return res.json() as Promise<T>;
}
