// `fixtures/golden.json` == `computeGolden()`. A missing file FAILS: a golden test that passes when the golden is absent
// guards nothing. The stored file is never regenerated from this package's own output; a change is a reviewed edit.

import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

import { computeGolden, GOLDEN_VERSION } from './golden.ts';
import { SDK_ROOT } from './helpers.ts';
import { test } from './node-test.ts';

const file = join(SDK_ROOT, 'fixtures/golden.json');

test('fixtures/golden.json matches the SDK: PDAs, instruction bytes and accounts, event and layout discriminators, tick math', async () => {
  assert.ok(existsSync(file), `${file} is missing`);
  const stored = JSON.parse(readFileSync(file, 'utf8')) as Record<string, unknown>;
  assert.equal(stored['version'], GOLDEN_VERSION);
  assert.deepEqual(await computeGolden(), stored);
});
