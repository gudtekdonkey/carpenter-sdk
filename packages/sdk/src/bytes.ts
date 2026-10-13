/**
 * Little-endian byte helpers. Every multi-byte integer on Solana (account fields, instruction
 * args, seeds) is little-endian; every 128-bit account field is two LE `u64` limbs, low first,
 * which is byte-identical to a LE `u128`.
 */

import { getAddressDecoder, getAddressEncoder, type Address } from '@solana/kit';

export const U64_MAX = (1n << 64n) - 1n;
export const U128_MAX = (1n << 128n) - 1n;
export const I128_MIN = -(1n << 127n);
export const I128_MAX = (1n << 127n) - 1n;

const addressEncoder = getAddressEncoder();
const addressDecoder = getAddressDecoder();

export function addressToBytes(a: Address): Uint8Array {
  return new Uint8Array(addressEncoder.encode(a));
}

export function bytesToAddress(b: Uint8Array): Address {
  if (b.length !== 32) throw new Error(`address must be 32 bytes, got ${b.length}`);
  return addressDecoder.decode(b);
}

export function concatBytes(...parts: readonly Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.length;
  }
  return out;
}

/** Bytewise comparison, the order sorts mints by. */
export function compareBytes(a: Uint8Array, b: Uint8Array): number {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) {
    const d = (a[i] as number) - (b[i] as number);
    if (d !== 0) return d;
  }
  return a.length - b.length;
}

export function equalBytes(a: Uint8Array, b: Uint8Array): boolean {
  return a.length === b.length && compareBytes(a, b) === 0;
}

export function hexToBytes(hex: string): Uint8Array {
  const h = hex.startsWith('0x') ? hex.slice(2) : hex;
  if (h.length % 2 !== 0 || /[^0-9a-fA-F]/.test(h)) throw new Error(`bad hex: ${hex}`);
  const out = new Uint8Array(h.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(h.slice(2 * i, 2 * i + 2), 16);
  return out;
}

export function bytesToHex(b: Uint8Array): string {
  let s = '';
  for (const x of b) s += x.toString(16).padStart(2, '0');
  return s;
}

export function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

export function bytesToBase64(b: Uint8Array): string {
  let bin = '';
  for (const x of b) bin += String.fromCharCode(x);
  return btoa(bin);
}

export function u16le(v: number): Uint8Array {
  const b = new Uint8Array(2);
  new DataView(b.buffer).setUint16(0, v, true);
  return b;
}

export function u32le(v: number): Uint8Array {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setUint32(0, v, true);
  return b;
}

export function i32le(v: number): Uint8Array {
  const b = new Uint8Array(4);
  new DataView(b.buffer).setInt32(0, v, true);
  return b;
}

export function u64le(v: bigint): Uint8Array {
  if (v < 0n || v > U64_MAX) throw new RangeError(`u64 out of range: ${v}`);
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, v, true);
  return b;
}

/** Read a LE unsigned integer of `len` bytes (any length; used for 128-bit fields). */
export function readUintLE(data: Uint8Array, offset: number, len: number): bigint {
  let v = 0n;
  for (let i = len - 1; i >= 0; i--) v = (v << 8n) | BigInt(data[offset + i] as number);
  return v;
}

export function readI128LE(data: Uint8Array, offset: number): bigint {
  const u = readUintLE(data, offset, 16);
  return u >= 1n << 127n ? u - (1n << 128n) : u;
}

/** Write a LE unsigned integer of `len` bytes. Throws when `v` does not fit. */
export function writeUintLE(out: Uint8Array, offset: number, len: number, v: bigint): void {
  if (v < 0n || v >> BigInt(8 * len) !== 0n) throw new RangeError(`u${8 * len} out of range: ${v}`);
  let x = v;
  for (let i = 0; i < len; i++) {
    out[offset + i] = Number(x & 0xffn);
    x >>= 8n;
  }
}

export function toTwos128(v: bigint): bigint {
  if (v < I128_MIN || v > I128_MAX) throw new RangeError(`i128 out of range: ${v}`);
  return v < 0n ? v + (1n << 128n) : v;
}
