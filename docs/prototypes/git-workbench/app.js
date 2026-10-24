const $ = id => document.getElementById(id);
const escapeHTML = s => String(s).replace(/[&<>"']/g, c => ({ '&':'&amp;', '<':'&lt;', '>':'&gt;', '"':'&quot;', "'":'&#39;' }[c]));
const paths = {
  loom:'M4 4h6v6H4V4Zm10 0h6v6h-6V4ZM4 14h6v6H4v-6Zm10 0h6v6h-6v-6ZM7 10v4m10-4v4M10 7h4m-4 10h4',
  enter:'M20 5v7H4m5-5-5 5 5 5', right:'M4 12h16m-6-6 6 6-6 6',
  copy:'M9 9h11v12H9V9ZM5 16H3V3h12v2', download:'M12 3v13m-5-5 5 5 5-5M4 17v4h16v-4',
  split:'M3 4h18v16H3V4Zm9 0v16M6 8h3m-3 4h3m6-4h3m-3 4h3', unified:'M4 4h16v16H4V4Zm4 4h8m-8 4h8m-8 4h5',
  file:'M5 3h9l5 5v13H5V3Zm9 0v6h5', markdown:'M3 5h18v14H3V5Zm4 10V9l3 4 3-4v6m3-5v5m-2-2 2 2 2-2',
  rust:'m12 3 2 2 3-1 1 3 3 1-1 3 1 3-3 1-1 3-3-1-2 2-2-2-3 1-1-3-3-1 1-3-1-3 3-1 1-3 3 1 2-2Zm-3 13V8h4a2 2 0 0 1 0 4H9m3 0 3 4',
  branch:'M6 3v12a3 3 0 0 0 3 3h6M6 7a2 2 0 1 0 0-4 2 2 0 0 0 0 4Zm12-4a2 2 0 1 0 0 4 2 2 0 0 0 0-4Zm0 4v4a4 4 0 0 1-4 4H6m12 1a2 2 0 1 0 0 4 2 2 0 0 0 0-4Z',
  chevron:'m8 10 4 4 4-4', refresh:'M20 7v5h-5M4 17v-5h5M6.1 7a7 7 0 0 1 11.5-2L20 8M4 16l2.4 3A7 7 0 0 0 18 17',
  upload:'M12 16V4m-5 5 5-5 5 5M5 16v4h14v-4', help:'M9 9a3 3 0 1 1 5 2c-1 1-2 1-2 3m0 3h.01M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0Z',
  files:'M8 4h10l3 3v14H8V4Zm10 0v4h3M4 17H2V1h12v1', search:'m21 21-5-5M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0Z',
  history:'M3 3v5h5M3 8a9 9 0 1 1 0 9m9-10v5l3 2', settings:'M9 3h6l1 3 3 1 2 5-2 5-3 1-1 3H9l-1-3-3-1-2-5 2-5 3-1 1-3Zm6 9a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z',
  tree:'M4 3v14h5m-5-9h5m2-3h9v6h-9V5Zm0 9h9v6h-9v-6Z', more:'M5 12h.01M12 12h.01M19 12h.01', folder:'M3 6h6l2 2h10v12H3V6Z',
  info:'M12 11v6m0-10h.01M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0Z', check:'m5 12 4 4L19 6', diff:'M5 3h10l4 4v14H5V3Zm10 0v5h4M8 12h8m-4-3v6m-4 3h8',
  close:'m6 6 12 12M6 18 18 6', layers:'m12 3 10 5-10 5L2 8l10-5Zm-10 9 10 5 10-5M2 16l10 5 10-5', up:'m6 15 6-6 6 6', down:'m6 9 6 6 6-6',
  undo:'M3 10h11a6 6 0 0 1 0 12M3 10l5-5m-5 5 5 5', plus:'M12 5v14M5 12h14', minus:'M5 12h14', sliders:'M4 7h7m4 0h5M4 17h3m4 0h9M11 4v6M7 14v6', filter:'M3 5h18M6 12h12M10 19h4',
  code:'m8 7-5 5 5 5m8-10 5 5-5 5M14 4l-4 16', stash:'M3 7h18v14H3V7Zm-1-4h20v4H2V3Zm7 9h6', external:'M14 3h7v7m0-7L11 13M9 3H3v18h18v-6', lock:'M5 11h14v10H5V11Zm3 0V7a4 4 0 0 1 8 0v4'
};
function icon(name, cls='') { return `<svg class="${cls}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="${paths[name] || paths.code}"/></svg>`; }
function hydrateIcons(root=document) {
  root.querySelectorAll('[data-icon]').forEach(e => e.innerHTML = icon(e.dataset.icon));
  root.querySelectorAll('button[aria-label]').forEach(button => { if(!button.title) button.title=button.getAttribute('aria-label'); });
}

const initialFiles = [
  { id:'panel', name:'panel.rs', dir:'src/ui/git', adds:24, dels:12, hunks:2, staged:[], type:'M' },
  { id:'diff', name:'diff_editor.rs', dir:'src/ui', adds:18, dels:6, hunks:2, staged:[], type:'M' },
  { id:'state', name:'state.rs', dir:'src/ui/git', adds:8, dels:2, hunks:1, staged:[0], type:'M' },
  { id:'actions', name:'actions.rs', dir:'src/features/git', adds:16, dels:0, hunks:1, staged:[], type:'A' },
  { id:'docs', name:'git.md', dir:'docs', adds:12, dels:4, hunks:1, staged:[], type:'M' },
  { id:'keymap', name:'keymap.rs', dir:'src/features/keymap', adds:5, dels:1, hunks:1, staged:[], type:'U', conflict:true },
  { id:'notes', name:'scratch.txt', dir:'notes', adds:4, dels:0, hunks:1, staged:[], type:'?' }
];
const typeInfo = { M:['text-[#ccae76]','已修改'], A:['text-accent','新增'], '?':['text-[#9aa6b5]','未跟踪'], U:['text-[#d29090]','存在冲突'] };
let files = structuredClone(initialFiles), active = 'panel', mode = 'split', hunk = 0, tree = false, branch = 'feature/git-workbench', outgoing = 2, historyTab = 'log', selectedCommit = '8f3a21c', scopeAll = false, toastTimer, modalAction, ignoredWhitespace = false, behind = 1, fetchCount = 0;
const narrow = matchMedia('(max-width: 1100px)');
const commits = [
  { hash:'8f3a21c', title:'refactor: unify source control state', author:'You', time:'12 分钟前', tag:'HEAD', files:['src/ui/git/state.rs','src/app/git_view.rs'], body:'Use a shared repository snapshot across the source control panel and editor.' },
  { hash:'6d9c4a2', title:'feat: add inline hunk actions', author:'You', time:'38 分钟前', tag:'', files:['src/ui/git/hunk_peek.rs','src/ui/diff_editor.rs'], body:'Stage and revert individual changes directly from the editor.' },
  { hash:'b4e7f10', title:'fix: preserve scroll position in diff viewer', author:'Alex Chen', time:'2 小时前', tag:'origin/main', files:['src/ui/diff_editor.rs'], body:'Keep the active change visible when switching the diff layout.' },
  { hash:'a2c8e56', title:'feat: repository switcher for workspaces', author:'Mia Liu', time:'昨天', tag:'', files:['src/app/workspace_git.rs','src/ui/git/panel.rs'], body:'Select repositories without leaving the current workspace.' },
  { hash:'91d0b38', title:'chore: update git runtime manifest', author:'Alex Chen', time:'昨天', tag:'', files:['scripts/package-git-runtime.ps1'], body:'Refresh the bundled Git runtime manifest.' }
];
const panelBefore = [
  'pub fn render_commit_panel(', '    state: &GitPanelState,', '    bounds: Rect,', ') -> impl IntoElement {', '    let changes = state.changes();', '', '    v_stack((', '        render_header("Source Control"),', '        scroll_view(', '            changes.iter().map(|file| {', '                render_file_row(file)', '            }),', '        ),', '        text_input("Commit message"),', '        icon_button(Icon::Check)', '            .on_click(Action::Commit),', '    ))', '    .gap(8)', '    .padding(12)', '}', '', '// Keep repository state in sync.', 'fn refresh_repository(cx: &mut Context) {', '    cx.dispatch(Action::Refresh);', '}'
];
const panelAfter = [
  'pub fn render_commit_panel(', '    state: &GitPanelState,', '    bounds: Rect,', ') -> impl IntoElement {', '    let changes = state.changes();', '', '    v_stack((', '        render_header("Commit"),', '        scroll_view(', '            changes.iter().map(|file| {', '                render_selectable_file(file)', '            }),', '        ).flex_1(),', '        commit_composer(state)', '            .placeholder("Describe your changes…")', '            .on_submit(Action::Commit),', '    ))', '    .gap(0)', '    .height(bounds.height)', '}', '', '// Keep repository state in sync.', 'fn refresh_repository(cx: &mut Context) {', '    cx.dispatch(Action::Refresh);', '}'
];
function contentFor(file) {
  if (file.id === 'panel') return [panelBefore, panelAfter];
  if (file.id === 'diff') return [ ['pub fn diff_toolbar(state: &DiffState) {','    row((', '        icon_button(Icon::ArrowUp),', '        icon_button(Icon::ArrowDown),','    ))','}', '', 'fn navigate_change(state: &mut DiffState) {', '    state.next_change();','}'], ['pub fn diff_toolbar(state: &DiffState) {','    row((', '        change_counter(state.current, state.total),', '        navigation_buttons(state),','    ))','}', '', 'fn navigate_change(state: &mut DiffState) {', '    state.next_change();', '    state.reveal_active_change();','}'] ];
  if (file.id === 'state') return [['pub struct GitPanelState {','    pub changes: Vec<FileChange>,','    pub message: String,','}'], ['pub struct GitPanelState {','    pub changes: Vec<FileChange>,','    pub message: String,','    pub selected_file: Option<PathBuf>,','    pub history_visible: bool,','    pub commit_in_progress: bool,','}']];
  if (file.id === 'keymap') return [['pub fn default_keymap() -> Keymap {','    Keymap::new()','        .bind("ctrl-enter", Action::Commit)','}'], ['pub fn default_keymap() -> Keymap {','    Keymap::new()','<<<<<<< HEAD','        .bind("ctrl-enter", Action::Commit)','=======','        .bind("ctrl-enter", Action::CommitAndPush)','>>>>>>> origin/main','}']];
  if (file.id === 'notes') return [[], ['TODO: git workbench','- conflict state','- empty state','- ahead / behind']];
  if (file.id === 'actions') return [[], ['use crate::features::git::RepositoryId;','','pub enum CommitAction {','    StageFile(PathBuf),','    UnstageFile(PathBuf),','    StageHunk { path: PathBuf, index: usize },','    Commit { message: String, amend: bool },','    Push { repository: RepositoryId },','}']];
  return [['# Git','','## Source Control','','Open the Source Control drawer.','Click a file to view all changes.','','## History','','Expand COMMITS to view recent history.'], ['# Git','','## Commit workspace','','Open the Commit tool window.','Select a file to preview its changes.','Stage individual files or change blocks.','','## History','','Browse commits in the Git tool window.','Select a commit to review files and details.']];
}
function highlight(code) {
  const tokens = /(\/\/.*$|"[^"\n]*"|\b(?:pub|fn|let|impl|mut|use|enum|struct|return)\b|\b\d+\b|\b[A-Za-z_][A-Za-z_0-9]*(?=\())/g;
  let result='', last=0;
  for (const match of code.matchAll(tokens)) { result += escapeHTML(code.slice(last,match.index)); const value=match[0]; const kind=value.startsWith('//')?'comment':value.startsWith('"')?'str':/^\d/.test(value)?'num':/^(pub|fn|let|impl|mut|use|enum|struct|return)$/.test(value)?'kw':'fn'; result+=`<span class="${kind}">${escapeHTML(value)}</span>`; last=match.index+value.length; }
  return result+escapeHTML(code.slice(last));
}
function alignedLines(before, after) {
  const dp = Array.from({length: before.length+1},()=>Array(after.length+1).fill(0));
  for(let i=before.length-1;i>=0;i--) for(let j=after.length-1;j>=0;j--) dp[i][j] = before[i]===after[j] ? 1+dp[i+1][j+1] : Math.max(dp[i+1][j],dp[i][j+1]);
  const raw=[]; let i=0,j=0;
  while(i<before.length || j<after.length) { if(i<before.length&&j<after.length&&before[i]===after[j]) {raw.push({old:before[i],new:after[j],oldN:++i,newN:++j,same:true});} else if(i<before.length&&(j===after.length||dp[i+1][j]>=dp[i][j+1])) {raw.push({old:before[i],oldN:++i});} else {raw.push({new:after[j],newN:++j});} }
  const aligned=[];
  for(let k=0;k<raw.length;) { if(raw[k].same) {aligned.push(raw[k++]); continue;} const dels=[],adds=[]; while(k<raw.length&&!raw[k].same) {const r=raw[k++]; (r.oldN?dels:adds).push(r);} for(let n=0;n<Math.max(dels.length,adds.length);n++) aligned.push({...dels[n],...adds[n],same:false}); }
  return aligned;
}
function toast(message, kind='ok') { $('toastText').textContent=message; $('toast').dataset.kind=kind; $('toast').firstElementChild.innerHTML=icon(kind==='error'?'info':'check'); $('toast').hidden=false; clearTimeout(toastTimer); toastTimer=setTimeout(()=>$('toast').hidden=true,3200); }
function showModal(title, content, label, action) { $('modalTitle').textContent=title; $('modalContent').innerHTML=content; $('modalConfirm').textContent=label; modalAction=action; $('modal').showModal(); }
$('modal').addEventListener('close',()=>{const action=modalAction; modalAction=null; if($('modal').returnValue==='confirm') action?.();});

function fileRow(f) {
  const [typeColor, typeTitle] = typeInfo[f.type] || typeInfo.M;
  const lead = f.conflict
    ? `<span class="w-[13px] shrink-0 text-[#d29090]" title="需要先解决冲突">${icon('info','!w-3.5 !h-3.5')}</span>`
    : `<input type="checkbox" class="file-check" data-stage-file="${f.id}" aria-label="暂存 ${f.name}" ${f.staged.length===f.hunks?'checked':''}>`;
  const kind = f.id==='docs'?'markdown':f.id==='notes'?'file':'rust';
  return `${tree?`<div class="text-[11px] text-[#627080] px-7 py-1">${escapeHTML(f.dir)}/</div>`:''}<div class="group flex items-center gap-2.5 px-4 h-[37px] ${active===f.id?'active-file':''}">${lead}<button data-file="${f.id}" class="flex-1 min-w-0 flex items-center gap-2 text-left h-full !bg-transparent"><span class="${f.type==='A'?'text-accent':'text-[#ad94d1]'}">${icon(kind,'!w-3.5 !h-3.5')}</span><span class="truncate text-[12px] ${active===f.id?'':'text-[#b6c0cb]'}">${f.name}</span>${f.staged.length&&f.staged.length<f.hunks?'<span class="text-[10px] text-accent">部分</span>':''}<span class="ml-auto text-[10px] ${typeColor}" title="${typeTitle}">${f.type}</span></button></div>`;
}
function renderFiles() {
  const query=$('fileSearch').value.toLowerCase();
  const filtered=files.filter(f=>(f.dir+'/'+f.name).toLowerCase().includes(query));
  const groups=[
    {name:'冲突', kind:'conflict', list:filtered.filter(f=>f.conflict)},
    {name:'已暂存', kind:'staged', list:filtered.filter(f=>!f.conflict&&f.staged.length===f.hunks)},
    {name:'未暂存', kind:'unstaged', list:filtered.filter(f=>!f.conflict&&f.staged.length<f.hunks)}
  ].filter(g=>g.kind!=='conflict'||files.some(f=>f.conflict));
  if(!files.length) $('fileList').innerHTML=`<div class="h-full flex flex-col items-center justify-center gap-2 text-[#657382] pb-10">${icon('check','!w-6 !h-6 text-accent')}<p class="m-0 text-[12px] text-[#aab4c0]">工作区干净</p><p class="m-0 text-[11px]">没有待提交的更改</p></div>`;
  else if(!filtered.length) $('fileList').innerHTML='<div class="px-4 py-8 text-center text-[11px] text-[#657382]">没有匹配的文件</div>';
  else $('fileList').innerHTML=groups.map(g=>{
    const action=g.kind==='conflict'?'':`<button data-stage-group="${g.kind==='staged'}" class="ml-auto text-[#657483] p-1 rounded" title="${g.kind==='staged'?'取消全部暂存':'暂存全部'}">${icon(g.kind==='staged'?'minus':'plus')}</button>`;
    const head=`<div class="flex items-center px-4 py-2 text-[11px] gap-2 ${g.kind==='conflict'?'text-[#d29090]':'text-[#929dab]'}"><span class="text-[#606c7a]">${icon('chevron','!w-3 !h-3')}</span>${g.name}<span class="rounded bg-[#ffffff07] px-1.5 text-[10px]">${g.list.length}</span>${action}</div>`;
    return head+(!g.list.length?'<div class="px-8 py-3 text-[11px] text-[#535f6e]">暂无文件</div>':g.list.map(fileRow).join(''));
  }).join('');
  $('fileTotal').textContent=files.length+' 个文件';
  updateCommitState();
}
function updateCommitState() {
  const selected=files.filter(f=>f.staged.length>0).length, conflicts=files.filter(f=>f.conflict).length;
  const valid=!conflicts && selected>0 && ($('commitMessage').value.trim().length>0 || $('amend').checked);
  $('commitSummary').textContent=selected+' 个文件已暂存'; $('commitBtn').disabled=!valid; $('commitMenuBtn').disabled=!valid; $('commitPushBtn').disabled=!valid;
  if(!valid) setCommitMenu(false);
  $('commitBtnText').textContent=$('amend').checked?'修订上次提交':'提交';
  const reason=conflicts?`还有 ${conflicts} 个冲突未解决`:!selected?'未暂存更改':!$('commitMessage').value.trim()&&!$('amend').checked?'缺少提交信息':'';
  $('commitHint').textContent=reason; $('commitHint').hidden=!reason;
  $('commitBtn').title=reason||'提交 · Ctrl + Enter'; $('commitPushBtn').title=reason||'提交并推送';
}
function setCommitMenu(open) { $('commitMenu').hidden=!open; $('commitMenuBtn').setAttribute('aria-expanded',open); }
function renderDiff() {
  const file=files.find(f=>f.id===active);
  $('diffPanel').hidden=!file; $('diffEmpty').hidden=!!file; $('diffEmptyText').textContent=files.length?'未选择文件':'工作区干净，没有可预览的更改';
  if(!file) { $('editorTab').textContent='更改预览'; $('diffStat').textContent=''; return; }
  $('editorTab').textContent=file.name; $('fileBreadcrumb').textContent=(file.dir+'/'+file.name).split('/').join(' / '); $('fileLanguage').textContent=file.id==='docs'?'Markdown':'Rust';
  $('diffStat').innerHTML=`<span class="text-accent">+${file.adds}</span><span class="text-[#d29090]">−${file.dels}</span>`;
  const staged=file.staged.length===file.hunks, eff=narrow.matches?'unified':mode;
  $('comparisonLabel').innerHTML=file.conflict?`<span class="text-[#d29090]">冲突 · HEAD ↔ origin/main</span>`:file.type==='?'?'<span>未跟踪的新文件</span>':`<span>HEAD</span>${icon('right','!w-3 !h-3')}<span>${staged?'暂存区':'工作区'}</span>`;
  $('hunkLabel').textContent=`${hunk+1} / ${file.hunks} 处改动`;
  updateHunkButton(file);
  $('splitBtn').setAttribute('aria-pressed',eff==='split'); $('unifiedBtn').setAttribute('aria-pressed',eff==='unified'); $('splitBtn').disabled=narrow.matches; $('splitBtn').title=narrow.matches?'窗口较窄，已自动使用统一视图':'并排视图';
  $('splitBtn').className='icon-button'; $('unifiedBtn').className='icon-button';
  const [before,after]=contentFor(file); const rows=alignedLines(before,after);
  $('diffHeaders').innerHTML=eff==='split'?`<div class="pl-4 flex items-center justify-between pr-3 border-r line"><span>HEAD <span class="text-[#535e6b] ml-2">${commits[0].hash}</span></span><span title="只读" aria-label="只读">${icon('lock','!w-3 !h-3 text-[#616d7a]')}</span></div><div class="pl-4 flex items-center pr-4"><span>${staged?'暂存区':'工作区'}</span></div>`:`<div class="pl-4 flex items-center gap-2"><span>HEAD</span>${icon('right','!w-3 !h-3')}<span>${staged?'暂存区':'工作区'}</span></div>`;
  function row(text,n,type) {return `<div class="code-row ${type}"><span class="ln">${n||''}</span><span class="sign">${type==='del'?icon('minus'):type==='add'?icon('plus'):''}</span><span>${text===undefined?' ':highlight(text)||' '}</span></div>`;}
  if(eff==='split') $('diffCode').innerHTML=`<div class="grid grid-cols-2 min-h-full min-w-[680px] code"><div class="border-r line pb-4 overflow-hidden">${rows.map(r=>row(r.old,r.oldN,r.same?'':r.oldN?'del':'void')).join('')}</div><div class="pb-4 overflow-hidden">${rows.map(r=>row(r.new,r.newN,r.same?'':r.newN?'add':'void')).join('')}</div></div>`;
  else $('diffCode').innerHTML=`<div class="code min-w-[640px] pb-4">${rows.map(r=>r.same?row(r.new,r.newN,''):(r.oldN?row(r.old,r.oldN,'del'):'')+(r.newN?row(r.new,r.newN,'add'):'')).join('')}</div>`;
  navigateHunk(false);
}
function updateHunkButton(file) {
  const conflict=!!file.conflict, staged=file.staged.includes(hunk), label=conflict?'标记为已解决':staged?'取消暂存此块':'暂存此块';
  $('stageHunkText').textContent=label; $('stageHunkBtn').title=label; $('stageHunkBtn').setAttribute('aria-label',label);
  $('stageHunkBtn').querySelector('[data-icon]').innerHTML=icon(conflict?'check':staged?'minus':'plus');
}
function navigateHunk(announce=true) {
  const file=files.find(f=>f.id===active); if(!file)return;
  $('hunkLabel').textContent=`${hunk+1} / ${file.hunks}`; updateHunkButton(file);
  const changed=$('diffCode').querySelectorAll('.code-row.add');
  const target=changed[Math.min(changed.length-1,hunk===0?0:Math.ceil(changed.length/2))];
  if(target && announce) $('diffCode').scrollTo({top:Math.max(0,target.offsetTop-$('diffCode').offsetTop-70),behavior:'smooth'});
}
function renderHistory() {
  if(historyTab!=='log') {
    const stashes=[['WIP: keyboard navigation','feature/git-workbench','2 小时前'],['Before upgrading dependencies','main','昨天']];
    const detailIcon=icon('external','!w-3.5 !h-3.5');
    if(historyTab==='stash') {
      $('historyBody').innerHTML=`<div class="p-4 w-full">${stashes.map((s,i)=>`<div class="flex items-center gap-3 py-3 border-b line">${icon('stash','text-[#8f9baa]')}<div><div class="text-[#c6cfd9]">${s[0]}</div><div class="text-[11px] muted mt-1">${s[1]} · ${s[2]}</div></div><button data-apply-stash="${i}" class="icon-button ml-auto" title="查看详情" aria-label="查看详情">${detailIcon}</button></div>`).join('')}</div>`;
    } else {
      const worktrees=[[branch,'D:/projects/loom',true],['fix/scroll-sync','D:/projects/loom-scroll',false]];
      $('historyBody').innerHTML=`<div class="p-4 w-full">${worktrees.map(([name,path,current])=>`<div class="flex gap-3 py-3 border-b line items-center">${icon('folder',current?'text-accent':'muted')}<div><div class="text-[#c6cfd9] flex items-center gap-2">${escapeHTML(name)}${current?icon('check','!w-3 !h-3 text-accent'):''}</div><div class="font-mono text-[11px] muted mt-1">${path}</div></div>${current?'':`<button data-open-worktree class="icon-button ml-auto" title="查看详情" aria-label="查看详情">${detailIcon}</button>`}</div>`).join('')}</div>`;
    }
    return;
  }
  const selected=commits.find(c=>c.hash===selectedCommit)||commits[0];
  $('historyBody').innerHTML=`<div class="branch-column w-[164px] shrink-0 border-r line p-3 scroll"><div class="text-[10px] tracking-wider text-[#606e7e] mb-2">分支</div><button data-log-branch="current" class="w-full flex items-center gap-2 rounded px-2 py-1.5 text-[11px] ${scopeAll?'text-[#8390a1]':'bg-[#82d8b40b] text-accent'}">${icon('branch')}<span class="truncate">${escapeHTML(branch)}</span></button><button data-log-branch="main" class="w-full flex gap-2 items-center rounded px-2 py-1.5 text-[11px] ${scopeAll?'bg-[#82d8b40b] text-accent':'text-[#8390a1]'}">${icon('branch')}main</button><div class="text-[10px] tracking-wider text-[#606e7e] mt-3 mb-2">远程</div><div class="text-[11px] text-[#8390a1] px-2 py-1.5 flex items-center gap-2">${icon('chevron','!w-3 !h-3')}<span>origin</span></div><div class="text-[11px] text-[#687789] px-6">main</div></div><div class="flex-1 min-w-0 scroll"><div class="h-7 flex items-center border-b line text-[10px] text-[#687484] px-4"><span>提交信息</span><span class="ml-auto mr-10">作者</span><span class="mr-3">时间</span></div>${commits.filter((c,i)=>scopeAll?i>=2:true).map(c=>`<button data-commit="${c.hash}" class="commit-row ${c.hash===selected.hash?'selected':''}"><span class="graph"><i></i></span><span class="truncate text-[12px] pr-3">${c.tag?`<span class="text-[10px] px-1.5 py-0.5 rounded mr-2 ${c.tag==='HEAD'?'text-accent bg-[#82d8b410]':'text-[#9eacbf] bg-[#ffffff08]'}">${c.tag}</span>`:''}${escapeHTML(c.title)}</span><span class="text-[11px] text-[#7e8b9b]">${c.author}</span><span class="text-[10px] text-[#657385]">${c.time}</span></button>`).join('')}</div><div class="history-detail w-[253px] shrink-0 border-l line p-4 scroll"><div class="flex justify-between items-center text-[11px]"><span class="font-mono text-accent">${selected.hash}</span><button id="copyHash" class="icon-button !w-6 !h-6" title="复制 Hash" aria-label="复制 Hash">${icon('copy','!w-3 !h-3')}</button></div><h3 class="text-[12px] text-[#ced8e1] mt-2 leading-5 mb-1">${escapeHTML(selected.title)}</h3><p class="text-[10px] muted mb-3">${selected.author} · ${selected.time}</p><div class="border-t line pt-2 text-[10px] text-[#687687] mb-1">${selected.files.length} 个更改文件</div>${selected.files.map(f=>`<div class="flex gap-2 py-1 text-[11px] text-[#a1afbd]">${icon('code','!w-3 !h-3 text-[#b39ad2]')}<span class="truncate" title="${escapeHTML(f)}">${escapeHTML(f.split('/').pop())}</span><span class="ml-auto text-[#bca26f] text-[10px]">M</span></div>`).join('')}</div>`;
}
function renderBranchList() {
  const names=[branch,'main','develop','fix/scroll-sync'].filter((n,i,a)=>a.indexOf(n)===i).filter(n=>n.toLowerCase().includes($('branchSearch').value.toLowerCase()));
  $('branchList').innerHTML='<div class="text-[10px] muted px-2 py-1">本地分支</div>'+names.map(n=>`<button data-branch="${escapeHTML(n)}" class="flex w-full text-left items-center gap-2 px-2 py-2 rounded ${n===branch?'text-accent':'text-[#b2bfca]'}">${icon(n===branch?'check':'branch')}<span>${escapeHTML(n)}</span>${n===branch?'<span class="ml-auto text-[10px] muted">当前</span>':''}</button>`).join('')+(names.length?'':'<p class="muted p-3">没有匹配的分支</p>');
}
function updateBranch() { $('currentBranch').textContent=branch; updateCommitState(); renderHistory(); }
function stageFile(id) { const f=files.find(f=>f.id===id); f.staged=f.staged.length===f.hunks?[]:Array.from({length:f.hunks},(_,i)=>i); renderFiles(); renderDiff(); }
function commit(andPush=false) {
  if($('commitBtn').disabled)return;
  const selected=files.filter(f=>f.staged.length>0), amend=$('amend').checked;
  const message=$('commitMessage').value.trim()||commits[0].title;
  const hash=Math.random().toString(16).slice(2,9).padEnd(7,'0');
  if(amend) commits.shift(); else outgoing++;
  commits.forEach(c=>{if(c.tag==='HEAD')c.tag='';});
  const entry={hash,title:message.split('\n')[0],author:'You',time:'刚刚',tag:'HEAD',files:selected.map(f=>f.dir+'/'+f.name),body:message};
  commits.unshift(entry); selectedCommit=hash;
  files=files.filter(f=>{if(f.staged.length===f.hunks)return false; f.hunks-=f.staged.length; f.staged=[]; return true;});
  active=files.some(f=>f.id===active)?active:files[0]?.id; hunk=0;
  $('commitMessage').value=''; $('amend').checked=false;
  renderFiles(); renderDiff(); renderHistory(); updatePushButton();
  toast(amend?'已修订上次提交':'已提交 '+selected.length+' 个文件');
  if(andPush) setTimeout(previewPush,350);
}
function updatePushButton() {
  $('pushLabel').textContent=outgoing||''; $('pushLabel').hidden=!outgoing;
  const label=outgoing?`推送 · ${outgoing} 个提交`:'已同步';
  $('pushBtn').title=label; $('pushBtn').setAttribute('aria-label',label);
  $('pushBtn').querySelector('[data-icon]').innerHTML=icon(outgoing?'upload':'check');
}
function previewPush() {
  if(!outgoing){toast('当前分支已与远程同步');return;}
  showModal('推送',`<div class="flex gap-2 items-center text-[12px] text-[#9aabba] mb-4">${icon('branch')}<span>${escapeHTML(branch)}</span>${icon('right','!w-3 !h-3')}<span>origin</span></div><div class="rounded-lg bg-[#191d23] p-3"><div class="text-accent mb-2">${outgoing} 个提交</div>${commits.slice(0,Math.min(outgoing,commits.length)).map(c=>`<div class="text-[12px] truncate"><span class="font-mono text-[#66788b] mr-2">${c.hash}</span>${escapeHTML(c.title)}</div>`).join('')}</div>`,'推送',()=>{outgoing=0;updatePushButton();toast('推送完成');});
}
function switchHistoryTab(tab) { historyTab=tab; document.querySelectorAll('[data-history-tab]').forEach(b=>b.className='h-full text-[12px] '+(b.dataset.historyTab===tab?'selected-tab':'text-[#7c8796]')); renderHistory(); }
function toggleHistory(force) { const visible=force??$('historyPanel').hidden; $('historyPanel').hidden=!visible; $('historyResizer').hidden=!visible; $('toggleHistory').setAttribute('aria-pressed',visible); }

document.addEventListener('click',event=>{
  const button=event.target.closest('button');
  if(button?.dataset.file) {active=button.dataset.file;hunk=0;renderFiles();renderDiff();}
  if(button?.dataset.stageGroup!==undefined) {const staged=button.dataset.stageGroup==='true';files.filter(f=>!f.conflict).forEach(f=>f.staged=staged?[]:Array.from({length:f.hunks},(_,i)=>i));renderFiles();renderDiff();}
  if(button?.dataset.commit) {selectedCommit=button.dataset.commit;renderHistory();}
  if(button?.dataset.historyTab) switchHistoryTab(button.dataset.historyTab);
  if(button?.dataset.branch) {const next=button.dataset.branch; if(next!==branch) showModal('切换分支',`<p>${files.length} 个未提交文件</p><div class="flex items-center gap-2 mt-2 text-accent">${icon('branch')}<span>${escapeHTML(next)}</span></div>`,'保留更改并切换',()=>{branch=next;updateBranch();toast('已切换到 '+branch);}); $('branchMenu').hidden=true; $('branchTrigger').setAttribute('aria-expanded','false');}
  if(button?.dataset.logBranch) {scopeAll=button.dataset.logBranch==='main'; $('scopeLabel').textContent=scopeAll?'main 分支':'当前分支'; selectedCommit=scopeAll?'b4e7f10':commits[0].hash;renderHistory();}
  if(button?.dataset.applyStash!==undefined) showModal('Stash',`<div class="flex items-center gap-2 mb-3">${icon('stash')}<span>${button.dataset.applyStash==='0'?'WIP: keyboard navigation':'Before upgrading dependencies'}</span></div><div class="rounded-lg bg-[#191d23] p-3 font-mono text-[12px] text-[#9bafbf]">src/ui/git/panel.rs<br>src/features/keymap/action.rs</div>`,'关闭',()=>{});
  if(button?.hasAttribute('data-open-worktree')) showModal('Worktree','<p>fix/scroll-sync</p><p class="font-mono text-[12px] text-[#8797a9]">D:/projects/loom-scroll</p>','关闭',()=>{});
  if(!event.target.closest('#commitMenuBtn')&&!event.target.closest('#commitMenu')) setCommitMenu(false);
  if(!event.target.closest('#branchTrigger')&&!event.target.closest('#branchMenu')) { $('branchMenu').hidden=true; $('branchTrigger').setAttribute('aria-expanded','false'); }
});
$('fileList').addEventListener('change',e=>{if(e.target.dataset.stageFile)stageFile(e.target.dataset.stageFile);});
$('fileSearch').addEventListener('input',renderFiles);
$('commitMessage').addEventListener('input',updateCommitState); $('amend').addEventListener('change',updateCommitState);
$('viewTree').onclick=()=>{tree=!tree;$('viewTree').setAttribute('aria-pressed',tree);renderFiles();};
$('splitBtn').onclick=()=>{mode='split';renderDiff();}; $('unifiedBtn').onclick=()=>{mode='unified';renderDiff();};
$('stageHunkBtn').onclick=()=>{const f=files.find(f=>f.id===active);if(!f)return; if(f.conflict){f.conflict=false;f.type='M';f.staged=Array.from({length:f.hunks},(_,i)=>i);renderFiles();renderDiff();toast('已将 '+f.name+' 标记为已解决');return;} f.staged=f.staged.includes(hunk)?f.staged.filter(i=>i!==hunk):[...f.staged,hunk];renderFiles();renderDiff();};
$('prevHunk').onclick=()=>{const f=files.find(f=>f.id===active);if(f){hunk=(hunk-1+f.hunks)%f.hunks;navigateHunk();}};
$('nextHunk').onclick=()=>{const f=files.find(f=>f.id===active);if(f){hunk=(hunk+1)%f.hunks;navigateHunk();}};
$('commitBtn').onclick=()=>commit(); $('commitPushBtn').onclick=()=>{setCommitMenu(false);commit(true);}; $('pushBtn').onclick=previewPush;
$('commitMenuBtn').onclick=()=>setCommitMenu($('commitMenu').hidden);
$('pullBtn').onclick=pull;
$('branchTrigger').onclick=()=>{const visible=$('branchMenu').hidden;$('branchMenu').hidden=!visible;$('branchTrigger').setAttribute('aria-expanded',visible);renderBranchList();if(visible)$('branchSearch').focus();};
$('branchSearch').oninput=renderBranchList;
function setBusy(button, label, busy) { button.disabled=busy; $('statusText').textContent=busy?label:'Git'; button.querySelector('svg')?.classList.toggle('animate-spin',busy); }
$('fetchBtn').onclick=()=>{const b=$('fetchBtn');setBusy(b,'Fetching…',true);setTimeout(()=>{setBusy(b,'',false);if(++fetchCount%3===0)toast('获取失败：无法连接到 origin，请检查网络后重试','error');else toast('已获取远程更新');},700);};
function updatePullButton() { $('pullLabel').textContent=behind||''; $('pullLabel').hidden=!behind; const label=behind?`拉取 · ${behind} 个提交`:'已是最新'; $('pullBtn').title=label; $('pullBtn').setAttribute('aria-label',label); }
function pull() { if(!behind){toast('已是最新');return;} const b=$('pullBtn');setBusy(b,'Pulling…',true);setTimeout(()=>{setBusy(b,'',false);behind=0;updatePullButton();toast('已拉取 1 个提交');},700); }
$('discardBtn').onclick=()=>{const f=files.find(f=>f.id===active); if(!f)return;showModal('还原 '+f.name,'<p>丢弃此文件的全部本地更改？</p>','还原',()=>{files=files.filter(x=>x.id!==f.id);active=files[0]?.id;hunk=0;renderFiles();renderDiff();toast('已还原 '+f.name);});};
$('diffOptions').onclick=()=>{ignoredWhitespace=!ignoredWhitespace;$('diffOptions').setAttribute('aria-pressed',ignoredWhitespace);$('diffOptions').title=ignoredWhitespace?'显示空白差异':'忽略空白差异';$('diffOptions').style.color=ignoredWhitespace?'#82d8b4':'';};
$('toggleHistory').onclick=()=>toggleHistory(); $('minimizeHistory').onclick=()=>toggleHistory(false);
$('historyScope').onclick=()=>{scopeAll=!scopeAll;$('scopeLabel').textContent=scopeAll?'main 分支':'当前分支';selectedCommit=scopeAll?'b4e7f10':commits[0].hash;renderHistory();};
$('closeDiff').onclick=()=>{active=null;renderFiles();renderDiff();};
document.addEventListener('keydown',e=>{if(e.key==='Escape'){setCommitMenu(false);$('branchMenu').hidden=true;$('branchTrigger').setAttribute('aria-expanded','false');}if((e.ctrlKey||e.metaKey)&&e.key==='Enter'&&!$('modal').open){e.preventDefault();commit();}});
document.addEventListener('click',async e=>{if(e.target.closest('#copyHash')) {try {await navigator.clipboard.writeText(selectedCommit);toast('已复制 '+selectedCommit);}catch{toast('Hash: '+selectedCommit);}}});
const panel=$('historyPanel'), resizer=$('historyResizer');
function setPanelHeight(h) { const max=Math.floor(panel.parentElement.clientHeight*0.6); panel.style.height=Math.max(140,Math.min(max,h))+'px'; }
resizer.addEventListener('pointerdown',e=>{
  resizer.setPointerCapture(e.pointerId); const startY=e.clientY, startH=panel.offsetHeight;
  const move=ev=>setPanelHeight(startH+startY-ev.clientY);
  const up=()=>{resizer.removeEventListener('pointermove',move);resizer.removeEventListener('pointerup',up);};
  resizer.addEventListener('pointermove',move); resizer.addEventListener('pointerup',up);
});
resizer.addEventListener('keydown',e=>{if(e.key==='ArrowUp'){e.preventDefault();setPanelHeight(panel.offsetHeight+24);}if(e.key==='ArrowDown'){e.preventDefault();setPanelHeight(panel.offsetHeight-24);}});
resizer.addEventListener('dblclick',()=>{panel.style.height='';});
narrow.addEventListener('change',renderDiff);
hydrateIcons(); renderFiles(); renderDiff(); renderHistory(); updatePullButton();
