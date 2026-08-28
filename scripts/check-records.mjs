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

function rangeFiles(base, head) {
  const emptyTree = execFileSync('git', ['hash-object', '-t', 'tree', '--stdin'], { encoding: 'utf8', input: '' }).trim();
  const start = /^0+$/.test(base) ? emptyTree : base;
  const output = execFileSync('git', ['diff', '--name-only', start, head], { encoding: 'utf8' });
  return output.split(/\r?\n/).filter(Boolean);
}

function filesToCheck(args) {
  const normalizedArgs = args.filter((argument) => argument !== '--');
  if (normalizedArgs.length === 0) {
    return stagedFiles();
  }
  if (
    normalizedArgs.length === 4
    && normalizedArgs[0] === '--base'
    && normalizedArgs[2] === '--head'
    && normalizedArgs[1]
    && normalizedArgs[3]
  ) {
    return rangeFiles(normalizedArgs[1], normalizedArgs[3]);
  }
  throw new Error('Use no arguments for staged files, or --base <revision> --head <revision> for a commit range.');
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    const files = filesToCheck(process.argv.slice(2));
    if (requiresRecord(files)) {
      console.error('제품 변경과 함께 PROJECT_LOG.txt, DECISIONS.txt 또는 CHANGELOG.md를 갱신하세요.');
      process.exit(1);
    }
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
