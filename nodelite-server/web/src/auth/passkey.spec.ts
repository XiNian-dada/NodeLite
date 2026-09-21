import { describe, expect, it } from 'vitest';
import { registrationRequest, supportsPasskeys } from './passkey';

describe('passkey browser helpers', () => {
  it('decodes WebAuthn challenge and credential IDs from base64url', () => {
    const request = registrationRequest({
      publicKey: {
        challenge: 'AQID',
        user: { id: 'BAUG', name: 'viewer', displayName: 'viewer' },
        excludeCredentials: [{ id: 'BwgJ', type: 'public-key' }],
      },
    });
    const publicKey = request.publicKey!;
    const user = publicKey.user;
    const excluded = publicKey.excludeCredentials?.[0];

    expect([...new Uint8Array(publicKey.challenge as ArrayBuffer)]).toEqual([1, 2, 3]);
    expect([...new Uint8Array(user.id as ArrayBuffer)]).toEqual([4, 5, 6]);
    expect([...new Uint8Array(excluded!.id as ArrayBuffer)]).toEqual([7, 8, 9]);
  });

  it('does not advertise passkeys when WebAuthn is unavailable', () => {
    expect(supportsPasskeys()).toBe(false);
  });
});
