// The `node:test` surface these suites were written against (`test(name, fn)`, `test(name, opts, fn)`,
// `t.diagnostic`, `t.skip`), over vitest. Only what the suites use.

import { test as vtest } from 'vitest';

export interface TestContext {
  /** Printed beside the test's result (`node:test`'s diagnostic line). */
  diagnostic(message: string): void;
  /** Marks the running test skipped and stops it. */
  skip(message?: string): void;
}

type Body = (t: TestContext) => unknown;
interface Options {
  skip?: boolean | string;
  timeout?: number;
}

export function test(name: string, a: Body | Options, b?: Body): void {
  const body = typeof a === 'function' ? a : b;
  const opts: Options = typeof a === 'function' ? {} : a;
  if (!body) throw new Error(`test ${name}: no body`);
  if (opts.skip) {
    vtest.skip(name, () => {});
    return;
  }
  vtest(
    name,
    async (ctx) => {
      await body({ diagnostic: (m) => console.log(`# ${name}: ${m}`), skip: (m) => ctx.skip(m) });
    },
    opts.timeout,
  );
}
