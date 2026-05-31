import { describe, expect, it } from 'vitest';
import { generatePassword, passwordFromBytes } from './password';

const CHARS = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#$%';

describe('passwordFromBytes', () => {
  it('maps each byte to charset[byte % len]', () => {
    const out = passwordFromBytes(new Uint8Array([0, 1, CHARS.length, CHARS.length + 2]));
    expect(out).toBe(`${CHARS[0]}${CHARS[1]}${CHARS[0]}${CHARS[2]}`);
  });

  it('produces a string the length of the byte array', () => {
    expect(passwordFromBytes(new Uint8Array(24))).toHaveLength(24);
  });
});

describe('generatePassword', () => {
  it('returns a string of the requested length from the charset', () => {
    const pw = generatePassword(20);
    expect(pw).toHaveLength(20);
    for (const ch of pw) expect(CHARS).toContain(ch);
  });

  it('is (practically) non-repeating across calls', () => {
    expect(generatePassword()).not.toBe(generatePassword());
  });
});
