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
  if(file.startsWith('crates/workspace-tests/'))return 'domain-verification';
  if(file.startsWith('crates/editor-core/'))return 'core';
  for(const l of ['app','domain','features','ui','infrastructure'])if(file.startsWith('src/'+l+'/'))return l;
  return 'legacy';
}

export function allowed(file, dependency) {
  const own=layer(file), root=dependency.replace('crate::','');
  if(own==='domain-verification')return ['domain','loom_editor_core','serde','serde_json'].includes(root);
  if(file.startsWith('crates/editor-core/tests/')&&root==='loom_editor_core')return true;
  if(own==='core')return ['identity','text','normal','commands','vim','buffer','change','editor','history','lines','selection','command','ex','exec','insert','key','motion','object','register','search','regex','unicode_segmentation','unicode_width'].includes(root);
  if(root==='loom_editor_core')return true;
  if(own==='app'||file==='src/main.rs')return true;
  if(own==='domain')return root==='domain'||(file==='src/domain/workspace_session.rs'&&['serde','serde_json'].includes(root));
  if(file.startsWith('src/features/explorer/'))return ['features','domain'].includes(root);
  if(file==='src/infrastructure/explorer_filesystem.rs')return ['features','domain','std::fs','std::process','windows_sys'].includes(root);
  if(file.startsWith('src/features/search/'))return ['features','domain','globset','regex','unicode_segmentation'].includes(root);
  if(file.startsWith('src/infrastructure/search_filesystem'))return ['features','domain','ignore','std::fs','std::process'].includes(root);
  if(file==='src/infrastructure/workspace_filesystem.rs')return ['infrastructure','domain','std::fs'].includes(root);
  if(file==='src/infrastructure/session_filesystem.rs')return ['infrastructure','domain','serde','serde_json','std::fs','std::process'].includes(root);
  if(file==='src/ui/editor/view.rs'&&root==='unicode_width')return true;
  if(file==='src/ui/explorer/context_menu.rs'&&root==='unicode_width')return true;
  if(own==='ui')return ['ui','domain','features','theme','fonts','file_icons','unicode_segmentation'].includes(root)||dependency==='lgui';
  if(own==='features')return ['features','domain','input'].includes(root);
  if(own==='infrastructure')return ['infrastructure','domain'].includes(root)||dependency==='lgui'||dependency==='std::thread';
  return own==='legacy'&&root==='domain';
}

export function structFields(source, name) {
  const ts=tokens(source), start=ts.findIndex((t,i)=>t==='struct'&&ts[i+1]===name);
  if(start<0)throw Error(`Missing ${name} declaration`);
  let depth=0;const fields=[];
  for(let i=start+2;i<ts.length;i++) {
    if(ts[i]==='{'){depth++;continue;}
    if(ts[i]==='}'&&--depth===0)break;
    if(depth===1&&ts[i+1]===':'&&/^[a-zA-Z_]\w*$/.test(ts[i]))fields.push(ts[i]);
  }
  return fields.sort();
}
export function stateFields(source) { return structFields(source, 'AppState'); }

export function verificationManifestErrors(source) {
  const block=source.split(/^\[dependencies\]\s*$/m)[1]?.split(/^\[/m)[0]??'';
  const names=[...block.matchAll(/^([a-zA-Z0-9_-]+)\s*=/gm)].map(m=>m[1]).sort();
  const expected=['loom-editor-core','serde','serde_json'].sort();
  if(JSON.stringify(names)!==JSON.stringify(expected)||/^\[[^\]]*dependencies[^\]]*\]$/m.test(source.replace(/^\[dependencies\]\s*$/m,''))||/\bpackage\s*=/.test(block))return ['Workspace domain verification may depend only on editor-core, serde and serde_json'];
  return [];
}

export function violations(sources, policy) {
  const problems=[];
  for(const [file,source] of Object.entries(sources)) {
    const approved=policy.exceptions[file]?.dependencies??[];
    const deps=dependencies(source);
    for(const dep of deps)if(!allowed(file,dep)&&!approved.includes(dep))problems.push(`${file}: unregistered dependency ${dep}`);
    for(const dep of approved)if(!deps.includes(dep))problems.push(`${file}: remove unused exception ${dep}`);
    const ts=tokens(source);
    if(layer(file)==='domain-verification') {
      if(policy.exceptions[file])problems.push(`${file}: domain verification cannot have migration exceptions`);
      if(ts.some((t,i)=>['include','include_str','include_bytes'].includes(t)&&ts[i+1]==='!')||(/#\s*\[\s*path\s*=/.test(source)&&(file!=='crates/workspace-tests/src/isolated.rs'||!/^#\[path = "\.\.\/\.\.\/\.\.\/src\/domain\/mod\.rs"\]$/m.test(source))))problems.push(`${file}: domain verification can only include the production domain entry point`);
    }
    if(layer(file)==='core') {
      if(policy.exceptions[file])problems.push(`${file}: editing core cannot have migration exceptions`);
      if(ts.some(t=>['AppState','AppRuntime','Workspace','ApplicationHandle','KeyboardEvent','Keystroke','UiRect','InputState'].includes(t)))problems.push(`${file}: editing core must not depend on application/platform state`);
      if(/#\s*\[\s*path\s*=/.test(source)||ts.some((t,i)=>['include','include_str','include_bytes'].includes(t)&&ts[i+1]==='!'))problems.push(`${file}: editing core cannot include sources outside its crate boundary`);
    }
    if(file.startsWith('src/features/search/')&&ts.some(t=>['AppState','AppRuntime','Workspace','ApplicationHandle','InputState','UiRect'].includes(t)))problems.push(file+': Search feature must be framework/root independent');
    if(file.startsWith('src/ui/search/')&&ts.some(t=>['AppState','AppRuntime','Workspace','ApplicationHandle'].includes(t)))problems.push(file+': Search view must use projections and commands');
    if(file.startsWith('src/ui/editor/')||file==='src/ui/editor_binding.rs') {
      if(ts.some(t=>['AppState','AppRuntime','Workspace','StateRoot','ApplicationHandle','Vim'].includes(t))||deps.some(d=>['crate::app','crate::state','crate::model','crate::git','crate::editor'].includes(d)))problems.push(file+': Editor view must use pane projections and commands');
      if(policy.exceptions[file])problems.push(file+': Editor view cannot have migration exceptions');
      if(/#\s*\[\s*path\s*=/.test(source)||ts.some((t,i)=>['include','include_str','include_bytes'].includes(t)&&ts[i+1]==='!'))problems.push(file+': Editor view cannot include root sources');
    }
    if(file.startsWith('src/features/explorer/')&&ts.some(t=>['AppState','AppRuntime','Workspace','ApplicationHandle','InputState','UiRect','State'].includes(t)))problems.push(file+': Explorer feature must be framework/root independent');
    if(file.startsWith('src/ui/explorer/')&&ts.some(t=>['AppState','AppRuntime','Workspace','ApplicationHandle'].includes(t)))problems.push(file+': Explorer view must use projections and commands');
    if(file.startsWith('src/ui/explorer/')&&policy.exceptions[file])problems.push(file+': Explorer view cannot have migration exceptions');
    if(file.startsWith('src/features/explorer/')&&policy.exceptions[file])problems.push(file+': Explorer feature cannot have migration exceptions');
    if((file.startsWith('src/features/search/')||file.startsWith('src/ui/search/'))&&policy.exceptions[file])problems.push(file+': Search cannot have migration exceptions');
    if(file==='src/model/workspace.rs'&&deps.some(d=>['crate::search','crate::search_actions','crate::features'].includes(d)))problems.push('Workspace must expose generic snapshots, not Search DTOs');
    if(layer(file)==='domain'&&(ts.some(t=>['super','AppState','AppRuntime'].includes(t))||/#\s*\[\s*path\s*=/.test(source)))problems.push(`${file}: domain must not escape through parent modules or root/runtime types`);
    if(layer(file)==='domain') {
      if(policy.exceptions[file])problems.push(`${file}: domain cannot have migration exceptions`);
      if(ts.some(t=>['UiRect','Color','ApplicationHandle','FileLoader','InputState'].includes(t)))problems.push(`${file}: domain must not contain framework or runtime types`);
      if(ts.some((t,i)=>['include','include_str','include_bytes'].includes(t)&&ts[i+1]==='!'))problems.push(`${file}: domain cannot include adapter sources`);
    }
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
  const editor=sources['src/app/state/editor.rs'];
  if(editor) {
    const actual=structFields(editor,'EditorState'),registered=Object.keys(policy.editor_state_ownership??{});
    for(const field of actual)if(!registered.includes(field))problems.push(`EditorState.${field}: no registered owner`);
    for(const field of registered)if(!actual.includes(field))problems.push(`EditorState.${field}: obsolete ownership entry`);
    if(tokens(editor).some(t=>['FileLoader','ApplicationHandle','PollControl','AppRuntime','AtomicBool'].includes(t)))problems.push('EditorState embeds a runtime service');
  }
  for(const [field,entry] of Object.entries(policy.editor_state_ownership??{}))if(!entry.owner||!entry.migrated_in)problems.push(`EditorState.${field}: incomplete ownership entry`);
  const workspace=sources['src/domain/workspace.rs'];
  if(workspace) {
    const actual=structFields(workspace,'Workspace'),registered=Object.keys(policy.workspace_core_ownership??{});
    for(const field of actual)if(!registered.includes(field))problems.push(`Workspace.${field}: no registered owner`);
    for(const field of registered)if(!actual.includes(field))problems.push(`Workspace.${field}: obsolete ownership entry`);
  }
  for(const [file,name,owner] of [['src/features/explorer/model.rs','ExplorerModel','features/explorer/model'],['src/ui/explorer/state.rs','ExplorerUiState','ui/explorer/state']]) {
    const source=sources[file];
    if(!source)continue;
    const actual=structFields(source,name),registered=Object.entries(policy.explorer_ownership??{}).filter(([,e])=>e.owner?.startsWith(owner)).map(([k])=>k);
    for(const field of actual)if(!registered.includes(field))problems.push(`${name}.${field}: no registered owner`);
    for(const field of registered)if(!actual.includes(field))problems.push(`${name}.${field}: obsolete ownership entry`);
  }
  const explorerModel=sources['src/features/explorer/model.rs'];
  if(explorerModel) {
    const ts=tokens(explorerModel),start=ts.findIndex((t,i)=>t==='struct'&&ts[i+1]==='ExplorerModel');
    let depth=0;
    for(let i=start+2;start>=0&&i<ts.length;i++){if(ts[i]==='{'){depth++;continue;}if(ts[i]==='}'&&--depth===0)break;if(depth===1&&ts[i]==='pub'){problems.push('ExplorerModel fields must stay private');break;}}
  }
  for(const [field,entry] of Object.entries(policy.explorer_ownership??{}))if(!entry.owner||!entry.migrated_in)problems.push(`Explorer.${field}: incomplete ownership entry`);
  for(const [field,entry] of Object.entries(policy.workspace_core_ownership??{}))if(!entry.owner||!entry.migrated_in)problems.push(`Workspace.${field}: incomplete ownership entry`);
  for(const [field,entry] of Object.entries(policy.state_ownership))if(!entry.owner||!entry.remove_in)problems.push(`AppState.${field}: incomplete ownership entry`);
  for(const [file,entry] of Object.entries(policy.exceptions))if(!entry.reason||!entry.remove_in)problems.push(`${file}: incomplete migration exception`);
  return problems;
}

export function readSources(root) {
  const result={};
  function visit(dir){for(const entry of fs.readdirSync(path.join(root,dir),{withFileTypes:true})){const file=dir+'/'+entry.name;if(entry.isDirectory())visit(file);else if(file.endsWith('.rs'))result[file]=fs.readFileSync(path.join(root,file),'utf8');}}
  visit('src');
  if(fs.existsSync(path.join(root,'crates/editor-core')))visit('crates/editor-core');
  if(fs.existsSync(path.join(root,'crates/workspace-tests')))visit('crates/workspace-tests');
  return result;
}

if(process.argv[1]&&path.resolve(process.argv[1])===fileURLToPath(import.meta.url)) {
  const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
  const policy=JSON.parse(fs.readFileSync(path.join(root,'docs/architecture/phase1-policy.json'),'utf8'));
  const sources=readSources(root),errors=violations(sources,policy);
  const verificationManifest=path.join(root,'crates/workspace-tests/Cargo.toml');
  if(fs.existsSync(verificationManifest))errors.push(...verificationManifestErrors(fs.readFileSync(verificationManifest,'utf8')));
  if(errors.length){console.error(errors.join('\n'));process.exitCode=1;}
  else console.log(`Architecture gate passed: ${Object.keys(sources).length} source files; ${Object.keys(policy.state_ownership).length} root, ${Object.keys(policy.editor_state_ownership??{}).length} editor and ${Object.keys(policy.workspace_core_ownership??{}).length} workspace owned fields; ${Object.keys(policy.exceptions).length} tracked legacy exception files.`);
}
