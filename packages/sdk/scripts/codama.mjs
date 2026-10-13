// `npm run generate`, step 2: idl/carpenter_amm.json → Codama → src/generated/client/carpenter_amm.
//
// Options:
// `syncPackageJson: false` (or the renderer rewrites package.json with its own @solana/kit range),
// `kitImportStrategy: 'rootOnly'` (everything from `@solana/kit` and its `program-client-core` subpath),
// `formatCode: false` (prettier is not a dependency), `importExtension: 'js'`, because this package
// compiles under NodeNext, where an extensionless relative import does not resolve.
// Never edit `src/generated/`: regenerate. Codama renames the `approvedHook` PDA helper (different seeds in two
//    instructions) to `initializePoolApprovedHook` and says so on stdout — expected, never "fixed" in the IDL.

import { createFromRoot } from 'codama';
import { rootNodeFromAnchor } from '@codama/nodes-from-anchor';
import { renderVisitor } from '@codama/renderers-js';
import { readFileSync, rmSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const pkg = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const PROGRAMS = ['carpenter_amm'];

/**
 * Drop the `pda` of any account whose own seeds name ITSELF, and print which. Anchor's IDL generator degrades a
 * seed read out of another account's DATA to that account's ADDRESS, which Codama reports as a cycle and refuses to
 * render; dropping it makes the account caller-supplied, the only correct client behaviour. `carpenter_amm` has none today; the check stays.
 */
function dropSelfReferentialPdas(idl, program) {
  const dropped = [];
  for (const ix of idl.instructions ?? []) {
    for (const acc of ix.accounts ?? []) {
      if (acc.pda?.seeds?.some((s) => s.kind === 'account' && s.path === acc.name)) {
        delete acc.pda;
        dropped.push(`${ix.name}.${acc.name}`);
      }
    }
  }
  if (dropped.length) console.log(`codama: ${program} — self-referential pda dropped, caller supplies it: ${dropped.join(', ')}`);
}

rmSync(join(pkg, 'src/generated/client'), { recursive: true, force: true });
for (const name of PROGRAMS) {
  const idl = JSON.parse(readFileSync(join(pkg, 'idl', `${name}.json`), 'utf8'));
  dropSelfReferentialPdas(idl, name);
  await createFromRoot(rootNodeFromAnchor(idl)).accept(
    renderVisitor(pkg, {
      generatedFolder: `src/generated/client/${name}`,
      syncPackageJson: false,
      kitImportStrategy: 'rootOnly',
      formatCode: false,
      importExtension: 'js',
    }),
  );
  console.log(`codama: ${name} → src/generated/client/${name} (IDL spec ${idl.metadata?.spec ?? '?'})`);
}
