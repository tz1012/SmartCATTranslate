import { readdir, readFile, stat, writeFile } from 'node:fs/promises';
import { basename, resolve } from 'node:path';
const root=resolve(process.argv[2]??'release-assets'); const repository=process.env.GITHUB_REPOSITORY; const tag=process.env.RELEASE_TAG??process.env.GITHUB_REF_NAME; const version=(process.env.RELEASE_VERSION??tag?.replace(/^app-v/,'')).trim();
if(!repository||!tag||!version) throw new Error('release_identity_missing');
const files=await walk(root); const platforms={};
for(const [target,key,pattern] of [['x86_64-pc-windows-msvc','windows-x86_64',/(?:setup|installer).*\.exe$/i],['x86_64-apple-darwin','darwin-x86_64',/\.app\.tar\.gz$/],['aarch64-apple-darwin','darwin-aarch64',/\.app\.tar\.gz$/]]) { const bundle=files.find((file)=>file.includes(target)&&pattern.test(file)&&files.includes(`${file}.sig`)); if(!bundle) throw new Error(`updater_bundle_or_signature_missing:${target}`); const signature=await readFile(`${bundle}.sig`,'utf8'); platforms[key]={signature:signature.trim(),url:`https://github.com/${repository}/releases/download/${tag}/${encodeURIComponent(basename(bundle))}`,size:(await stat(bundle)).size}; }
await writeFile('latest.json',`${JSON.stringify({version,notes:'See CHANGELOG.md in the release assets.',pub_date:new Date().toISOString(),platforms},null,2)}\n`);
async function walk(dir){const out=[];for(const entry of await readdir(dir,{withFileTypes:true})){const path=`${dir}/${entry.name}`;if(entry.isDirectory())out.push(...await walk(path));else out.push(path);}return out;}
