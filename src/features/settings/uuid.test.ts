import { describe, expect, it } from 'vitest';
import { createUuidV4 } from './uuid';

describe('createUuidV4', () => {
  it('creates an RFC 4122 version 4 UUID when randomUUID is unavailable', () => {
    const cryptoFallback = {
      getRandomValues(bytes: Uint8Array) {
        bytes.set([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
        return bytes;
      },
    };

    expect(createUuidV4(cryptoFallback)).toBe('00010203-0405-4607-8809-0a0b0c0d0e0f');
  });
});
