import { ApiError } from '@/api/client';

/**
 * Extract a human message from a settings/alerts mutation failure. The
 * server returns `{ ok:false, message }` JSON on non-2xx, which api() wraps
 * as ApiError(status, body). Prefer that message; fall back otherwise.
 */
export function messageFromError(error: unknown, fallback: string): string {
  if (error instanceof ApiError) {
    try {
      const parsed = JSON.parse(error.body) as { message?: unknown };
      if (typeof parsed.message === 'string' && parsed.message) return parsed.message;
    } catch {
      // body wasn't JSON — fall through
    }
    if (error.body) return error.body;
  }
  if (error instanceof Error && error.message) return error.message;
  return fallback;
}
