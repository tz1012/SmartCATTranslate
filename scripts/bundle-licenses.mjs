import { cp, mkdir, readFile, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
const output = resolve(process.argv[2] ?? 'artifacts/licenses');
await mkdir(output, { recursive: true });
await cp('tests/fixtures/fonts/LICENSE.txt', join(output, 'NotoSans-LICENSE.txt'));
const manifest = JSON.parse(await readFile('src-tauri/resources/codex-runtime.json', 'utf8'));
await writeFile(join(output, 'CODEX-RUNTIME-LICENSE.json'), `${JSON.stringify({ version:manifest.version, license:manifest.license }, null, 2)}\n`);
await writeFile(join(output, 'README.txt'), 'Dependency license details are represented in the accompanying npm and Cargo CycloneDX SBOMs. PDFium is not bundled.\n');
