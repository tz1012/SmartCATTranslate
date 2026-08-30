import { existsSync,readdirSync,readFileSync,statSync } from 'node:fs';
import { resolve } from 'node:path';

const roots=process.argv.slice(2); const files=[];
function collect(path){if(!existsSync(path))return;const stat=statSync(path);if(stat.isDirectory()){for(const name of readdirSync(path))collect(resolve(path,name));}else if(stat.size<=5*1024*1024)files.push(path);}
for(const root of roots.length?roots:['test-results/logs'])collect(resolve(root));
const rules=[
  ['bearer token',/\bBearer\s+[A-Za-z0-9._~+\/-]{12,}/i],
  ['token prefix',/\b(?:sk|ghp|github_pat|xox[baprs])[-_][A-Za-z0-9_-]{10,}/i],
  ['email address',/\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b/i],
  ['Windows user path',/[A-Z]:\\Users\\[^\\\s]+\\/i],
  ['Unix home path',/(?:\/Users|\/home)\/[^/\s]+\//],
  ['seeded source canary',/SMARTCAT_PRIVATE_SOURCE_CANARY|민감원문_검사_표식/i],
];
const findings=[];
for(const file of files){const content=readFileSync(file,'utf8');for(const[name,pattern]of rules){if(pattern.test(content))findings.push(`${file}: ${name}`);}}
if(findings.length){console.error(`privacy scan failed (${findings.length})\n${findings.join('\n')}`);process.exitCode=1;}else{console.log(`privacy scan passed (${files.length} files)`);}
