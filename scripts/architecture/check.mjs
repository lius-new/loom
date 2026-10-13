import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// Lexical gate, including cfg(test). It is intentionally not a Rust type resolver.
export function tokens(source) {
  const out=[];
  for(let i=0;i<source.length;) {
    if(/\s/.test(source[i])) {i++;continue;}
    if(source.startsWith('//',i)) {i=source.indexOf('\n',i);if(i<0)break;continue;}
    if(source.startsWith('/*',i)) {
      i+=2;let depth=1;
      while(i<source.length&&depth) {
        if(source.startsWith('/*',i)){depth++;i+=2;}
        else if(source.startsWith('*/',i)){depth--;i+=2;} else i++;
      }
      if(depth)throw Error('Unclosed block comment');
      continue;
    }
    const raw=/^(?:b|c)?r(#{0,255})"/.exec(source.slice(i));
    if(raw){const end=source.indexOf('"'+raw[1],i+raw[0].length);if(end<0)throw Error('Unclosed raw string');i=end+1+raw[1].length;continue;}
    const str=/^(?:b|c)?"/.exec(source.slice(i));
    if(str){i+=str[0].length;while(i<source.length){if(source[i]==='\\'){i+=2;continue;}if(source[i++]==='"')break;}continue;}
    const char=/^(?:b)?'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^'\\])'/u.exec(source.slice(i));
    if(char){i+=char[0].length;continue;}
    const word=/^[a-zA-Z_][a-zA-Z_0-9]*/.exec(source.slice(i));
    if(word){out.push(word[0]);i+=word[0].length;continue;}
    if(source.startsWith('::',i)){out.push('::');i+=2;continue;}
    out.push(source[i++]);
  }
  return out;
}

export function dependencies(source) {
  const ts=tokens(source), found=new Set();
  for(let i=0;i<ts.length;i++) {
    if(ts[i]==='use'&&/^[a-zA-Z_]\w*$/.test(ts[i+1]??'')&&!['crate','std','super','self'].includes(ts[i+1]))found.add(ts[i+1]);
    if(ts[i]==='crate'&&ts[i+1]==='::') {
      if(ts[i+2]==='{') {
        let level=0;
        for(let j=i+2;j<ts.length;j++) {
          if(ts[j]==='{'){level++;continue;}
          if(ts[j]==='}'&&--level===0)break;
          if(level===1&&(ts[j-1]==='{'||ts[j-1]===',')&&/^[a-zA-Z_]\w*$/.test(ts[j]))found.add('crate::'+ts[j]);
        }
      } else if(/^[a-zA-Z_]\w*$/.test(ts[i+2]??''))found.add('crate::'+ts[i+2]);
    }
    if(['lgui','image','notify','conpty','portable_pty','windows_sys','libc'].includes(ts[i])&&ts[i+1]==='::')found.add(ts[i]);
    if(ts[i]==='std'&&ts[i+1]==='::'&&['fs','process','thread','net'].includes(ts[i+2]))found.add('std::'+ts[i+2]);
    if(ts[i]==='std'&&ts[i+1]==='::'&&ts[i+2]==='{') {
      let depth=1;
      for(let j=i+3;j<ts.length&&depth;j++) {
        if(ts[j]==='{')depth++;
        else if(ts[j]==='}')depth--;
        else if(depth===1&&['fs','process','thread','net'].includes(ts[j])&&(ts[j-1]==='{'||ts[j-1]===','))found.add('std::'+ts[j]);
      }
    }
  }
  return [...found].sort();
}

export function layer(file) {
  for(const l of ['app','domain','features','ui','infrastructure'])if(file.startsWith('src/'+l+'/'))return l;
  return 'legacy';
}

export function allowed(file, dependency) {
  const own=layer(file), root=dependency.replace('crate::','');
  if(own==='app'||file==='src/main.rs')return true;
  if(own==='domain')return root==='domain';
  if(file.startsWith('src/features/search/'))return ['features','domain','globset','regex','unicode_segmentation'].includes(root);
  if(file.startsWith('src/infrastructure/search_filesystem'))return ['features','domain','ignore','std::fs','std::process'].includes(root);
  if(own==='ui')return ['ui','domain','features','theme','fonts','file_icons','unicode_segmentation'].includes(root)||dependency==='lgui';
  if(own==='features')return ['features','domain','input'].includes(root);
  if(own==='infrastructure')return ['infrastructure','domain'].includes(root)||dependency==='lgui'||dependency==='std::thread';
  return own==='legacy'&&root==='domain';
}

export function stateFields(source) {
  const ts=tokens(source), start=ts.findIndex((t,i)=>t==='struct'&&ts[i+1]==='AppState');
  if(start<0)throw Error('Missing AppState declaration');
  let depth=0;const fields=[];
  for(let i=start+2;i<ts.length;i++) {
    if(ts[i]==='{'){depth++;continue;}
    if(ts[i]==='}'&&--depth===0)break;
    if(depth===1&&ts[i+1]===':'&&/^[a-zA-Z_]\w*$/.test(ts[i]))fields.push(ts[i]);
  }
  return fields.sort();
}

export function violations(sources, policy) {
  const problems=[];
  for(const [file,source] of Object.entries(sources)) {
    const approved=policy.exceptions[file]?.dependencies??[];
    const deps=dependencies(source);
    for(const dep of deps)if(!allowed(file,dep)&&!approved.includes(dep))problems.push(`${file}: unregistered dependency ${dep}`);
    for(const dep of approved)if(!deps.includes(dep))problems.push(`${file}: remove unused exception ${dep}`);
    const ts=tokens(source);
    if(file.startsWith('src/features/search/')&&ts.some(t=>['AppState','AppRuntime','Workspace','ApplicationHandle','InputState','UiRect'].includes(t)))problems.push(file+': Search feature must be framework/root independent');
    if(file.startsWith('src/ui/search/')&&ts.some(t=>['AppState','AppRuntime','Workspace','ApplicationHandle'].includes(t)))problems.push(file+': Search view must use projections and commands');
    if((file.startsWith('src/features/search/')||file.startsWith('src/ui/search/'))&&policy.exceptions[file])problems.push(file+': Search cannot have migration exceptions');
    if(file==='src/model/workspace.rs'&&deps.some(d=>['crate::search','crate::search_actions','crate::features'].includes(d)))problems.push('Workspace must expose generic snapshots, not Search DTOs');
    if(layer(file)==='domain'&&(ts.some(t=>['super','AppState','AppRuntime'].includes(t))||/#\s*\[\s*path\s*=/.test(source)))problems.push(`${file}: domain must not escape through parent modules or root/runtime types`);
    if(layer(file)==='ui'&&ts.includes('AppState')&&!policy.root_state_adapters.includes(file))problems.push(`${file}: new root-state adapter`);
    if(/\.\s*runtime\s*\(/.test(ts.join(' '))&&!policy.runtime_bridges.includes(file))problems.push(`${file}: unregistered runtime bridge`);
  }
  for(const file of policy.root_state_adapters)if(!sources[file]||!tokens(sources[file]).includes('AppState'))problems.push(`${file}: obsolete root-state adapter entry`);
  for(const file of policy.runtime_bridges)if(!sources[file]||!/\.\s*runtime\s*\(/.test(tokens(sources[file]).join(' ')))problems.push(`${file}: obsolete runtime bridge entry`);
  for(const file of Object.keys(policy.exceptions))if(!sources[file])problems.push(`${file}: obsolete exception entry`);
  const state=sources['src/app/state.rs'];
  if(state) {
    const actual=stateFields(state),registered=Object.keys(policy.state_ownership).sort();
    for(const field of actual)if(!registered.includes(field))problems.push(`AppState.${field}: no registered owner`);
    for(const field of registered)if(!actual.includes(field))problems.push(`AppState.${field}: obsolete ownership entry`);
    const body=state.slice(state.indexOf('pub struct AppState'),state.indexOf('\nimpl AppState'));
    if(/\b(?:FileLoader|ApplicationHandle|PollControl)\b/.test(tokens(body).join(' ')))problems.push('AppState embeds a runtime service');
  }
  const search=sources['src/ui/search/state.rs']??sources['src/search_actions.rs'];
  if(search) {
    const start=search.indexOf('pub struct SearchUiState'),end=search.indexOf('\nimpl SearchUiState',start);
    if(start>=0&&/\b(?:AtomicBool|ApplicationHandle|FileLoader|AppRuntime)\b/.test(tokens(search.slice(start,end)).join(' ')))problems.push('SearchState embeds a runtime service');
  }
  for(const [field,entry] of Object.entries(policy.state_ownership))if(!entry.owner||!entry.remove_in)problems.push(`AppState.${field}: incomplete ownership entry`);
  for(const [file,entry] of Object.entries(policy.exceptions))if(!entry.reason||!entry.remove_in)problems.push(`${file}: incomplete migration exception`);
  return problems;
}

export function readSources(root) {
  const result={};
  function visit(dir){for(const entry of fs.readdirSync(path.join(root,dir),{withFileTypes:true})){const file=dir+'/'+entry.name;if(entry.isDirectory())visit(file);else if(file.endsWith('.rs'))result[file]=fs.readFileSync(path.join(root,file),'utf8');}}
  visit('src');return result;
}

if(process.argv[1]&&path.resolve(process.argv[1])===fileURLToPath(import.meta.url)) {
  const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
  const policy=JSON.parse(fs.readFileSync(path.join(root,'docs/architecture/phase1-policy.json'),'utf8'));
  const sources=readSources(root),errors=violations(sources,policy);
  if(errors.length){console.error(errors.join('\n'));process.exitCode=1;}
  else console.log(`Architecture gate passed: ${Object.keys(sources).length} source files; ${Object.keys(policy.state_ownership).length} owned fields; ${Object.keys(policy.exceptions).length} tracked legacy exception files.`);
}
