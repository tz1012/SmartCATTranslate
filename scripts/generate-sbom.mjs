import { spawn } from 'node:child_process';
import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';

const ecosystem = arg('--ecosystem');
const output = resolve(arg('--output'));
let components;
if (ecosystem === 'npm') {
  const listed = JSON.parse(await run('pnpm', ['list', '--json', '--depth', 'Infinity']));
  components = flattenPnpm(listed[0] ?? {});
} else if (ecosystem === 'cargo') {
  const metadata = JSON.parse(await run('cargo', ['metadata', '--locked', '--format-version', '1', '--manifest-path', 'src-tauri/Cargo.toml']));
  components = metadata.packages.map((pkg) => ({ type: 'library', name: pkg.name, version: pkg.version, purl: `pkg:cargo/${pkg.name}@${pkg.version}` }));
} else throw new Error('ecosystem_invalid');
components = [...new Map(components.map((component) => [`${component.name}@${component.version}`, component])).values()].sort((a,b) => `${a.name}@${a.version}`.localeCompare(`${b.name}@${b.version}`));
if (components.length < 2) throw new Error('sbom_components_insufficient');
await mkdir(dirname(output), { recursive: true });
await writeFile(output, `${JSON.stringify({ bomFormat:'CycloneDX', specVersion:'1.6', version:1, metadata:{component:{type:'application',name:'SmartCAT Translate'}}, components }, null, 2)}\n`);

function flattenPnpm(root) { const out=[]; const visit=(deps={})=>Object.entries(deps).forEach(([name,value])=>{ const version=String(value.version??'unknown').replace(/^npm:/,''); out.push({type:'library',name,version,purl:`pkg:npm/${encodeURIComponent(name)}@${encodeURIComponent(version)}`}); visit(value.dependencies); visit(value.devDependencies); }); visit(root.dependencies); visit(root.devDependencies); return out; }
function run(command,args) { return new Promise((ok,bad)=>{ const child=spawn(command,args,{cwd:process.cwd(),shell:false,windowsHide:true,stdio:['ignore','pipe','inherit']}); let text=''; child.stdout.on('data',(chunk)=>text+=chunk); child.on('error',bad); child.on('exit',(code)=>code===0?ok(text):bad(new Error(`${command}_failed`))); }); }
function arg(name) { const index=process.argv.indexOf(name); const value=process.argv[index+1]; if(index<0||!value||value.startsWith('--')) throw new Error('argument_missing'); return value; }
