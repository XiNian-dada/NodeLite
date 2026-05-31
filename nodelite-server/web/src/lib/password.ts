/**
 * Password generator, ported from assets/index.html:2357. The byte→char
 * mapping is pure (testable with injected bytes); generatePassword wraps it
 * with crypto.getRandomValues for the suggest-password helper.
 */

const CHARS = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#$%';

/** Map random bytes to the legacy charset. Pure. */
export function passwordFromBytes(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => CHARS[b % CHARS.length]).join('');
}

export function generatePassword(length = 24): string {
  const bytes = new Uint8Array(length);
  crypto.getRandomValues(bytes);
  return passwordFromBytes(bytes);
}
