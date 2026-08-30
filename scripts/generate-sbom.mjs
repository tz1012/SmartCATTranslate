import { spawn } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { validateCycloneDx16, validateSpdxExpression } from './license-policy.mjs';

const ecosystem = arg('--ecosystem');
const output = resolve(arg('--output'));
const exceptions = JSON.parse(await readFile('scripts/release-license-exceptions.json', 'utf8'));
let components;
if (ecosystem === 'npm') {
  const groups = JSON.parse(await run('pnpm', ['licenses', 'list', '--json', '--prod']));
  components = Object.values(groups).flatMap((entries) => entries.flatMap((entry) => entry.versions.map((version) => component('npm', entry.name, version, entry.license))));
} else if (ecosystem === 'cargo') {
  const metadata = JSON.parse(await run('cargo', ['metadata', '--locked', '--format-version', '1', '--filter-platform', rustTarget(), '--manifest-path', 'src-tauri/Cargo.toml']));
  components = metadata.packages.filter((pkg) => !metadata.workspace_members.includes(pkg.id)).map((pkg) => component('cargo', pkg.name, pkg.version, pkg.license));
} else throw new Error('ecosystem_invalid');
components = [...new Map(components.map((value) => [`${value.name}@${value.version}`, value])).values()].sort((a,b) => `${a.name}@${a.version}`.localeCompare(`${b.name}@${b.version}`));
if (components.length < 2) throw new Error('sbom_components_insufficient');
await mkdir(dirname(output), { recursive: true });
const jsonText = `${JSON.stringify({ $schema:'http://cyclonedx.org/schema/bom-1.6.schema.json', bomFormat:'CycloneDX', specVersion:'1.6', version:1, metadata:{component:{type:'application',name:'SmartCAT Translate'}}, components }, null, 2)}\n`;
await validateCycloneDx16(jsonText);
await writeFile(output, jsonText);

function component(kind, name, version, expression) {
  const exception = exceptions[`${kind}:${name}@${version}`];
  const license = validateSpdxExpression(exception?.license || expression, `${kind}:${name}@${version}`);
  return { type:'library', name, version, purl:`pkg:${kind}/${encodeURIComponent(name)}@${encodeURIComponent(version)}`, licenses:[{ expression:license }] };
}
function rustTarget() { return process.env.RELEASE_TARGET || (process.platform === 'win32' ? 'x86_64-pc-windows-msvc' : process.arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin'); }
function run(command,args) { const useShell=process.platform==='win32'&&command==='pnpm'; return new Promise((ok,bad)=>{ const child=spawn(command,args,{cwd:process.cwd(),shell:useShell,windowsHide:true,stdio:['ignore','pipe','inherit']}); let text=''; child.stdout.on('data',(chunk)=>text+=chunk); child.on('error',bad); child.on('exit',(code)=>code===0?ok(text):bad(new Error(`${command}_failed`))); }); }
function arg(name) { const index=process.argv.indexOf(name); const value=process.argv[index+1]; if(index<0||!value||value.startsWith('--')) throw new Error('argument_missing'); return value; }
