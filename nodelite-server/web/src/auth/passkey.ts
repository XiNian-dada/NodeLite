import type { PasskeyCredentialResponse, PasskeyRegistrationOptions } from '@/api';

type SerializedCredentialDescriptor = Record<string, unknown> & { id: string };

function base64UrlToBuffer(value: string): ArrayBuffer {
  const base64 = value.replace(/-/g, '+').replace(/_/g, '/');
  const padded = base64.padEnd(Math.ceil(base64.length / 4) * 4, '=');
  const binary = window.atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes.buffer;
}

function bufferToBase64Url(value: ArrayBuffer): string {
  const bytes = new Uint8Array(value);
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return window.btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/g, '');
}

function decodeDescriptors(value: unknown): PublicKeyCredentialDescriptor[] | undefined {
  if (!Array.isArray(value)) return undefined;
  return value.map((descriptor) => {
    const serialised = descriptor as SerializedCredentialDescriptor;
    return { ...serialised, id: base64UrlToBuffer(serialised.id) } as PublicKeyCredentialDescriptor;
  });
}

/** True only when the current browser exposes the WebAuthn credential APIs. */
export function supportsPasskeys(): boolean {
  return (
    typeof window !== 'undefined' &&
    typeof window.PublicKeyCredential !== 'undefined' &&
    typeof navigator.credentials?.create === 'function'
  );
}

/** Converts the server's base64url options into browser WebAuthn creation options. */
export function registrationRequest(
  options: PasskeyRegistrationOptions,
): CredentialCreationOptions {
  const publicKey = options.publicKey;
  const user = publicKey.user;
  return {
    publicKey: {
      ...publicKey,
      challenge: base64UrlToBuffer(publicKey.challenge),
      user: { ...user, id: base64UrlToBuffer(user.id) },
      excludeCredentials: decodeDescriptors(publicKey.excludeCredentials),
    } as PublicKeyCredentialCreationOptions,
  };
}

/** Serialises a browser registration response into the protocol accepted by webauthn-rs. */
export function registrationResponse(credential: PublicKeyCredential): PasskeyCredentialResponse {
  const response = credential.response as AuthenticatorAttestationResponse;
  return {
    id: credential.id,
    rawId: bufferToBase64Url(credential.rawId),
    type: credential.type,
    response: {
      attestationObject: bufferToBase64Url(response.attestationObject),
      clientDataJSON: bufferToBase64Url(response.clientDataJSON),
      transports: response.getTransports?.(),
    },
    clientExtensionResults: credential.getClientExtensionResults(),
  };
}

/** Opens the platform/browser passkey UI and returns its JSON-safe registration result. */
export async function createPasskey(
  options: PasskeyRegistrationOptions,
): Promise<PasskeyCredentialResponse> {
  if (!supportsPasskeys()) throw new Error('Passkeys are not supported by this browser');
  const credential = await navigator.credentials.create(registrationRequest(options));
  if (!(credential instanceof PublicKeyCredential)) {
    throw new Error('Passkey registration was cancelled');
  }
  return registrationResponse(credential);
}
