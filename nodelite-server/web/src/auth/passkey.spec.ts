import { afterEach, describe, expect, it, vi } from 'vitest';
import { authenticatePasskey, registrationRequest, supportsPasskeys } from './passkey';

describe('passkey browser helpers', () => {
  afterEach(() => vi.unstubAllGlobals());
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

  it('decodes an assertion challenge and serialises the signed response', async () => {
    class CredentialMock {
      id = 'credential';
      rawId = new Uint8Array([1, 2]).buffer;
      type = 'public-key';
      response = {
        authenticatorData: new Uint8Array([3]).buffer,
        clientDataJSON: new Uint8Array([4]).buffer,
        signature: new Uint8Array([5]).buffer,
        userHandle: null,
      };
      getClientExtensionResults() {
        return {};
      }
    }
    const get = vi.fn().mockResolvedValue(new CredentialMock());
    vi.stubGlobal('PublicKeyCredential', CredentialMock);
    vi.stubGlobal('navigator', { credentials: { get } });
    const result = await authenticatePasskey({
      publicKey: {
        challenge: 'AQID',
        allowCredentials: [{ id: 'BAUG', type: 'public-key' }],
      },
    });
    expect([...new Uint8Array(get.mock.calls[0]?.[0].publicKey.challenge)]).toEqual([1, 2, 3]);
    expect(result.response).toMatchObject({
      authenticatorData: 'Aw',
      clientDataJSON: 'BA',
      signature: 'BQ',
    });
  });
});
