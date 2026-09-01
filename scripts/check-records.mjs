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

function isCommit(revision) {
  try {
    execFileSync('git', ['cat-file', '-e', `${revision}^{commit}`], { stdio: 'pipe' });
    return true;
  } catch {
    return false;
  }
}

function resolveCommit(revision) {
  return execFileSync('git', ['rev-parse', '--verify', `${revision}^{commit}`], {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();
}

function fallbackMergeBase(fallbackBase, head) {
  if (!fallbackBase || !isCommit(fallbackBase) || !isCommit(head)) {
    throw new Error('The explicit base is unavailable and the fallback base or head is not a valid commit.');
  }

  const resolvedFallback = resolveCommit(fallbackBase);
  const resolvedHead = resolveCommit(head);
  let mergeBase;
  try {
    mergeBase = execFileSync('git', ['merge-base', resolvedFallback, resolvedHead], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    }).trim();
  } catch {
    throw new Error('The fallback base and head do not have a merge base.');
  }
  if (!mergeBase) {
    throw new Error('The fallback base and head do not have a merge base.');
  }
  if (mergeBase === resolvedHead) {
    throw new Error('The fallback merge base resolves to head; refusing an empty enforcement range.');
  }
  return { head: resolvedHead, start: mergeBase };
}

function rangeFiles(base, head, fallbackBase) {
  const emptyTree = execFileSync('git', ['hash-object', '-t', 'tree', '--stdin'], { encoding: 'utf8', input: '' }).trim();
  let start = emptyTree;
  let end = head;
  if (!/^0+$/.test(base)) {
    if (isCommit(base)) {
      start = base;
    } else {
      ({ head: end, start } = fallbackMergeBase(fallbackBase, head));
    }
  }
  const output = execFileSync('git', ['diff', '--name-only', start, end], { encoding: 'utf8' });
  return output.split(/\r?\n/).filter(Boolean);
}

function filesToCheck(args) {
  const normalizedArgs = args.filter((argument) => argument !== '--');
  if (normalizedArgs.length === 0) {
    return stagedFiles();
  }
  if (
    (normalizedArgs.length === 4 || normalizedArgs.length === 6)
    && normalizedArgs[0] === '--base'
    && normalizedArgs[2] === '--head'
    && normalizedArgs[1]
    && normalizedArgs[3]
    && (
      normalizedArgs.length === 4
      || (normalizedArgs[4] === '--fallback-base' && normalizedArgs[5])
    )
  ) {
    return rangeFiles(normalizedArgs[1], normalizedArgs[3], normalizedArgs[5]);
  }
  throw new Error('Use no arguments for staged files, or --base <revision> --head <revision> [--fallback-base <revision>] for a commit range.');
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
