import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AUTH_TIMESTAMP_KEY } from '@/auth/expiry';
import { ApiAbortError, ApiError, LOGOUT_PATH, VERIFY_2FA_PATH, api } from './client';

function makeResponse(init: {
  status?: number;
  ok?: boolean;
  redirected?: boolean;
  url?: string;
  body?: unknown;
  contentType?: string;
}): Response {
  const status = init.status ?? 200;
  const ok = init.ok ?? (status >= 200 && status < 300);
  const headers = new Headers();
  if (init.contentType !== undefined) {
    headers.set('content-type', init.contentType);
  }
  const bodyText =
    typeof init.body === 'string' ? init.body : JSON.stringify(init.body ?? null);
  return {
    status,
    ok,
    redirected: init.redirected ?? false,
    url: init.url ?? 'http://localhost/api/anything',
    headers,
    text: () => Promise.resolve(bodyText),
    json: () => Promise.resolve(init.body),
  } as unknown as Response;
}

describe('api client', () => {
  let assignSpy: ReturnType<typeof vi.fn>;
  let originalLocation: Location;
  let fetchMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    window.localStorage.clear();
    originalLocation = window.location;
    assignSpy = vi.fn();
    Object.defineProperty(window, 'location', {
      configurable: true,
      value: { assign: assignSpy, href: originalLocation.href },
    });
    fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
  });

  afterEach(() => {
    window.localStorage.clear();
    Object.defineProperty(window, 'location', {
      configurable: true,
      value: originalLocation,
    });
    vi.unstubAllGlobals();
  });

  it('redirects to /verify-2fa when fetch followed a redirect to it', async () => {
    fetchMock.mockResolvedValueOnce(
      makeResponse({
        redirected: true,
        url: 'http://localhost/verify-2fa',
        contentType: 'text/html',
      }),
    );

    await expect(api('/api/bootstrap')).rejects.toBeInstanceOf(ApiAbortError);
    expect(assignSpy).toHaveBeenCalledWith(VERIFY_2FA_PATH);
  });

  it('clears auth timestamp and redirects to logout on 401', async () => {
    window.localStorage.setItem(AUTH_TIMESTAMP_KEY, '12345');
    fetchMock.mockResolvedValueOnce(
      makeResponse({ status: 401, ok: false, body: 'unauthorized' }),
    );

    await expect(api('/api/overview')).rejects.toBeInstanceOf(ApiAbortError);
    expect(window.localStorage.getItem(AUTH_TIMESTAMP_KEY)).toBeNull();
    expect(assignSpy).toHaveBeenCalledWith(LOGOUT_PATH);
  });

  it('throws ApiError for non-JSON 200 response', async () => {
    fetchMock.mockResolvedValueOnce(
      makeResponse({ contentType: 'text/html', body: '<html>...</html>' }),
    );

    await expect(api('/api/bootstrap')).rejects.toMatchObject({
      name: 'ApiError',
      status: 200,
    });
    expect(assignSpy).not.toHaveBeenCalled();
  });

  it('returns parsed JSON for a JSON 200', async () => {
    fetchMock.mockResolvedValueOnce(
      makeResponse({
        contentType: 'application/json; charset=utf-8',
        body: { hello: 'world' },
      }),
    );

    await expect(api<{ hello: string }>('/api/bootstrap')).resolves.toEqual({
      hello: 'world',
    });
  });

  it('throws ApiError for non-401 error responses', async () => {
    fetchMock.mockResolvedValueOnce(
      makeResponse({ status: 404, ok: false, body: 'not found' }),
    );

    await expect(api('/api/nodes/missing')).rejects.toMatchObject({
      name: 'ApiError',
      status: 404,
      body: 'not found',
    });
    expect(assignSpy).not.toHaveBeenCalled();
  });

  it('does not treat unrelated redirects as 2FA', async () => {
    fetchMock.mockResolvedValueOnce(
      makeResponse({
        redirected: true,
        url: 'http://localhost/api/overview',
        contentType: 'application/json',
        body: { ok: true },
      }),
    );

    await expect(api<{ ok: boolean }>('/api/overview')).resolves.toEqual({
      ok: true,
    });
    expect(assignSpy).not.toHaveBeenCalled();
  });

  it('exports ApiError as a thrown subclass with status + body', async () => {
    fetchMock.mockResolvedValueOnce(
      makeResponse({ status: 503, ok: false, body: 'down' }),
    );

    try {
      await api('/api/overview');
      throw new Error('expected throw');
    } catch (e) {
      expect(e).toBeInstanceOf(ApiError);
      const err = e as ApiError;
      expect(err.status).toBe(503);
      expect(err.body).toBe('down');
    }
  });
});
