import { existsSync, lstatSync, readdirSync, readFileSync } from 'node:fs';
import { relative, resolve } from 'node:path';

const MAX_FILE_BYTES = 5 * 1024 * 1024;
const roots = process.argv.slice(2);
const requestedRoots = roots.length ? roots : ['scripts/fixtures/captured-logs'];
const files = [];
const collectionFailures = [];

const displayPath = (path) => {
  const value = relative(process.cwd(), path);
  return value.startsWith('..') ? '<external>' : value;
};

function collect(path) {
  if (!existsSync(path)) {
    collectionFailures.push(`${displayPath(path)}: missing scan input`);
    return;
  }
  const stat = lstatSync(path);
  if (stat.isSymbolicLink()) {
    collectionFailures.push(`${displayPath(path)}: symbolic link scan input`);
    return;
  }
  if (stat.isDirectory()) {
    for (const name of readdirSync(path)) collect(resolve(path, name));
    return;
  }
  if (stat.size > MAX_FILE_BYTES) {
    collectionFailures.push(`${displayPath(path)}: scan input exceeds size limit`);
    return;
  }
  files.push(path);
}

for (const root of requestedRoots) collect(resolve(root));
if (!files.length) collectionFailures.push('no readable captured logs or records');

const rules = [
  ['bearer token', /\bBearer\s+[A-Za-z0-9._~+\/-]{12,}/i],
  ['token prefix', /\b(?:sk|ghp|github_pat|xox[baprs])[-_][A-Za-z0-9_-]{10,}/i],
  ['email address', /\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b/i],
  ['Windows user path', /[A-Z]:\\Users\\[^\\\s]+\\/i],
  ['Unix home path', /(?:\/Users|\/home)\/[^/\s]+\//],
  ['seeded source canary', /SMARTCAT_PRIVATE_SOURCE_CANARY|민감원문_검사_표식/i],
];
const findings = [...collectionFailures];
for (const file of files) {
  const content = readFileSync(file, 'utf8');
  for (const [name, pattern] of rules) {
    if (pattern.test(content)) findings.push(`${displayPath(file)}: ${name}`);
  }
}
if (findings.length) {
  console.error(`privacy scan failed (${findings.length})\n${findings.join('\n')}`);
  process.exitCode = 1;
} else {
  console.log(`privacy scan passed (${files.length} files)`);
}
