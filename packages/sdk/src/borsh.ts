/**
 * The Borsh subset every instruction and event here uses: fixed integers (LE), bool, fixed byte arrays,
 * `Vec<u8>` / `String` (u32 LE length prefix), `Option<T>` (0/1 tag).
 * Strict: the reader refuses trailing bytes when asked to finish, as the program does.
 */

import type { Address } from '@solana/kit';

import { addressToBytes, bytesToAddress, readI128LE, readUintLE, toTwos128, writeUintLE } from './bytes.ts';

export class BorshWriter {
  private buf: Uint8Array = new Uint8Array(256);
  private len = 0;

  private reserve(n: number): number {
    if (this.len + n > this.buf.length) {
      const next = new Uint8Array(Math.max(this.buf.length * 2, this.len + n));
      next.set(this.buf.subarray(0, this.len));
      this.buf = next;
    }
    const at = this.len;
    this.len += n;
    return at;
  }

  raw(bytes: Uint8Array): this {
    // `reserve` FIRST, on its own line: it may REPLACE `this.buf` with a grown one, and in
    // `this.buf.set(bytes, this.reserve(n))` the receiver `this.buf` is evaluated BEFORE the call,
    // so the write would land in the OLD 256-byte buffer — `RangeError: offset is out of bounds` on any
    // instruction wider than 256 B.
    const at = this.reserve(bytes.length);
    this.buf.set(bytes, at);
    return this;
  }

  private uint(n: number, v: bigint | number): this {
    // Same ordering trap as `raw`: arguments evaluate left to right, so `this.buf` would be captured
    // before `reserve` could grow it.
    const at = this.reserve(n);
    writeUintLE(this.buf, at, n, BigInt(v));
    return this;
  }

  u8(v: number): this {
    return this.uint(1, v);
  }
  u16(v: number): this {
    return this.uint(2, v);
  }
  u32(v: number): this {
    return this.uint(4, v);
  }
  u64(v: bigint | number): this {
    return this.uint(8, v);
  }
  u128(v: bigint): this {
    return this.uint(16, v);
  }
  i32(v: number): this {
    if (!Number.isInteger(v) || v < -(2 ** 31) || v >= 2 ** 31) throw new RangeError(`i32 out of range: ${v}`);
    return this.uint(4, v < 0 ? BigInt(v) + (1n << 32n) : BigInt(v));
  }
  i64(v: bigint): this {
    if (v < -(1n << 63n) || v >= 1n << 63n) throw new RangeError(`i64 out of range: ${v}`);
    return this.uint(8, v < 0n ? v + (1n << 64n) : v);
  }
  i128(v: bigint): this {
    return this.uint(16, toTwos128(v));
  }
  bool(v: boolean): this {
    return this.u8(v ? 1 : 0);
  }
  /** A fixed `[u8; N]`; throws when the length differs. */
  fixed(bytes: Uint8Array, n: number): this {
    if (bytes.length !== n) throw new RangeError(`expected ${n} bytes, got ${bytes.length}`);
    return this.raw(bytes);
  }
  pubkey(a: Address): this {
    return this.raw(addressToBytes(a));
  }
  /** A Borsh `Vec<[u8; 32]>` — a u32 LE count, then each 32-byte item. Throws on an item that is not 32 bytes. */
  vecBytes32(items: readonly Uint8Array[]): this {
    this.u32(items.length);
    for (const it of items) this.fixed(it, 32);
    return this;
  }
  /** A Borsh `Vec<SlotArg>`: a u32 LE count, then each slot `bps u16 ‖ platform u8 ‖ key [u8; 32]` (35 B), for a
   *  hook SDK's own instruction arguments. Throws on a key that is not 32 bytes. */
  vecSlotArgs(items: readonly { bps: number; platform: number; key: Uint8Array }[]): this {
    this.u32(items.length);
    for (const it of items) this.u16(it.bps).u8(it.platform).fixed(it.key, 32);
    return this;
  }
  vecU8(bytes: Uint8Array): this {
    this.u32(bytes.length);
    return this.raw(bytes);
  }
  string(s: string): this {
    return this.vecU8(new TextEncoder().encode(s));
  }
  optionU32(v: number | null | undefined): this {
    if (v === null || v === undefined) return this.u8(0);
    return this.u8(1).u32(v);
  }
  /** A `[u16; 4]`; throws unless exactly four. */
  u16x4(v: readonly number[]): this {
    if (v.length !== 4) throw new RangeError(`expected 4 u16s, got ${v.length}`);
    return this.u16(v[0]!).u16(v[1]!).u16(v[2]!).u16(v[3]!);
  }

  toBytes(): Uint8Array {
    return this.buf.slice(0, this.len);
  }
}

export class BorshReader {
  offset = 0;
  private readonly data: Uint8Array;
  constructor(data: Uint8Array) {
    this.data = data;
  }

  private take(n: number): number {
    if (this.offset + n > this.data.length) throw new RangeError(`borsh: read past end (${this.offset}+${n} > ${this.data.length})`);
    const at = this.offset;
    this.offset += n;
    return at;
  }

  u8(): number {
    return this.data[this.take(1)] as number;
  }
  u16(): number {
    return Number(readUintLE(this.data, this.take(2), 2));
  }
  u32(): number {
    return Number(readUintLE(this.data, this.take(4), 4));
  }
  i32(): number {
    const u = Number(readUintLE(this.data, this.take(4), 4));
    return u >= 2 ** 31 ? u - 2 ** 32 : u;
  }
  u64(): bigint {
    return readUintLE(this.data, this.take(8), 8);
  }
  i64(): bigint {
    const u = readUintLE(this.data, this.take(8), 8);
    return u >= 1n << 63n ? u - (1n << 64n) : u;
  }
  u128(): bigint {
    return readUintLE(this.data, this.take(16), 16);
  }
  i128(): bigint {
    return readI128LE(this.data, this.take(16));
  }
  bool(): boolean {
    const b = this.u8();
    if (b > 1) throw new RangeError(`borsh: bad bool ${b}`);
    return b === 1;
  }
  fixed(n: number): Uint8Array {
    const at = this.take(n);
    return this.data.slice(at, at + n);
  }
  pubkey(): Address {
    return bytesToAddress(this.fixed(32));
  }
  /** A Borsh `Vec<SlotArg>` (`bps u16 ‖ platform u8 ‖ key [u8; 32]` each). */
  vecSlotArgs(): Array<{ bps: number; platform: number; key: Uint8Array }> {
    const n = this.u32();
    return Array.from({ length: n }, () => ({ bps: this.u16(), platform: this.u8(), key: this.fixed(32) }));
  }
  vecU8(): Uint8Array {
    return this.fixed(this.u32());
  }
  string(): string {
    return new TextDecoder('utf-8', { fatal: true }).decode(this.vecU8());
  }
  option<T>(inner: () => T): T | null {
    const tag = this.u8();
    if (tag === 0) return null;
    if (tag !== 1) throw new RangeError(`borsh: bad option tag ${tag}`);
    return inner();
  }
  vec<T>(inner: () => T): T[] {
    const n = this.u32();
    const out: T[] = [];
    for (let i = 0; i < n; i++) out.push(inner());
    return out;
  }
  remaining(): number {
    return this.data.length - this.offset;
  }
  /** Strict decode: trailing bytes are an error. */
  finish(): void {
    if (this.remaining() !== 0) throw new RangeError(`borsh: ${this.remaining()} trailing bytes`);
  }
}
