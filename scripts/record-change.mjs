import { appendFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const secretPattern = /(secret|token|credential|password|api[_-]?key)/i;
const bearerTokenPattern = /\bbearer\s+\S+/i;
const absoluteUserPath = /(?:[A-Za-z]:[\\/]|\\\\|\/(?:Users|home)\/)/;
const summaryPattern = /^(?:feature|fix|chore|docs|test|build|security): [a-z0-9]+(?:-[a-z0-9]+){0,11}$/;
const diagnosticCommandPattern = /^(?:pnpm|npm|node|cargo|git|vitest)(?: [A-Za-z0-9_./:@=+\-]+)*$/i;
const diagnosticResultPattern = /^(?:(?:\d+\/\d+ )?(?:passed|failed|skipped)|exit \d+)$/i;
const maximumRecordFieldLength = 160;

function formatSeoulTime(now) {
  const parts = new Intl.DateTimeFormat('en-GB', {
    timeZone: 'Asia/Seoul',
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  }).formatToParts(now);
  const values = Object.fromEntries(parts.map(({ type, value }) => [type, value]));
  return `${values.year}-${values.month}-${values.day} ${values.hour}:${values.minute}`;
}

function relativePaths(files, repositoryRoot) {
  return [...new Set(files.map((file) => path.relative(repositoryRoot, path.resolve(repositoryRoot, file))))]
    .filter((file) => file && !file.startsWith('..') && !path.isAbsolute(file))
    .map((file) => file.replaceAll('\\', '/'))
    .filter((file) => !file.startsWith('sources/') && !secretPattern.test(file))
    .sort((left, right) => left.localeCompare(right));
}

function validateRecordText(field, value, isAllowed) {
  if (typeof value !== 'string' || value.length === 0) {
    throw new Error(`${field} is required.`);
  }
  if (value.length > maximumRecordFieldLength) {
    throw new Error(`${field} is too long.`);
  }
  if (/[\x00-\x1F\x7F]/.test(value)) {
    throw new Error(`${field} contains a control character.`);
  }
  if (absoluteUserPath.test(value)) {
    throw new Error(`${field} contains an absolute path.`);
  }
  if (secretPattern.test(value) || bearerTokenPattern.test(value)) {
    throw new Error(`${field} contains a secret.`);
  }
  if (!isAllowed(value)) {
    throw new Error(`${field} must contain approved record metadata.`);
  }
  return value;
}

function isDiagnosticMetadata(value) {
  return value.split(/\s*(?:&&|;)\s*/).every((part) =>
    diagnosticCommandPattern.test(part) || diagnosticResultPattern.test(part),
  );
}

export function buildRecordEntry({ summary, tests, files, repositoryRoot = process.cwd(), now = new Date() }) {
  const safeSummary = validateRecordText('Summary', summary, (value) => summaryPattern.test(value));
  const safeTests = validateRecordText('Tests', tests, isDiagnosticMetadata);

  const changedFiles = relativePaths(files, repositoryRoot);
  return `\n[${formatSeoulTime(now)} Asia/Seoul] 변경 기록\n- 요약: ${safeSummary}\n- 변경 파일: ${changedFiles.join(', ')}\n- 검증: ${safeTests}\n`;
}

function gitOutput(repositoryRoot, args) {
  return execFileSync('git', args, { cwd: repositoryRoot, encoding: 'utf8' })
    .split(/\r?\n/)
    .filter(Boolean);
}

function parseArguments(args) {
  const parsed = { filesFromGit: false };
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    if (argument === '--') {
      continue;
    }
    if (argument === '--files-from-git') {
      parsed.filesFromGit = true;
    } else if (argument === '--summary' || argument === '--tests') {
      parsed[argument.slice(2)] = args[index + 1];
      index += 1;
    } else {
      throw new Error(`Unknown argument: ${argument}`);
    }
  }
  if (!parsed.summary || !parsed.tests || !parsed.filesFromGit) {
    throw new Error('Use --summary, --tests, and --files-from-git.');
  }
  return parsed;
}

function recordFromGit(parsed) {
  const repositoryRoot = execFileSync('git', ['rev-parse', '--show-toplevel'], { encoding: 'utf8' }).trim();
  const tracked = gitOutput(repositoryRoot, ['diff', '--name-only', 'HEAD']);
  const untracked = gitOutput(repositoryRoot, ['ls-files', '--others', '--exclude-standard']);
  const entry = buildRecordEntry({
    summary: parsed.summary,
    tests: parsed.tests,
    files: [...tracked, ...untracked],
    repositoryRoot,
  });
  appendFileSync(path.join(repositoryRoot, 'PROJECT_LOG.txt'), entry, 'utf8');
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    recordFromGit(parseArguments(process.argv.slice(2)));
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
