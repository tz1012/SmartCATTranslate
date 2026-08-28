import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const recordFiles = new Set(['PROJECT_LOG.txt', 'DECISIONS.txt', 'CHANGELOG.md']);
const ignoredPrefixes = ['docs/', '.github/', 'scripts/'];

export function requiresRecord(files) {
  const changedProduct = files.some((file) =>
    !recordFiles.has(file) && !ignoredPrefixes.some((prefix) => file.startsWith(prefix))
  );
  const changedRecord = files.some((file) => recordFiles.has(file));
  return changedProduct && !changedRecord;
}

function stagedFiles() {
  const output = execFileSync('git', ['diff', '--cached', '--name-only'], { encoding: 'utf8' });
  return output.split(/\r?\n/).filter(Boolean);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const files = stagedFiles();
  if (requiresRecord(files)) {
    console.error('제품 변경과 함께 PROJECT_LOG.txt, DECISIONS.txt 또는 CHANGELOG.md를 갱신하세요.');
    process.exit(1);
  }
}
