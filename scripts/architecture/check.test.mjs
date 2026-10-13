import test from 'node:test';
import assert from 'node:assert/strict';
import {dependencies,stateFields,structFields,violations,verificationManifestErrors} from './check.mjs';
const policy=()=>({exceptions:{},root_state_adapters:[],runtime_bridges:[],state_ownership:{}});

test('workspace, document and pane rules cannot regain I/O, UI, Git or adapter exceptions',()=>{
  for(const file of ['src/domain/workspace.rs','src/domain/document.rs','src/domain/pane_layout.rs']) {
    for(const source of ['use crate::git::DiffRequest;', 'use crate::app::workspace::Workspace;', 'use lgui::prelude::*;', 'use std::{fs, thread};', 'pub struct View { area: UiRect }', 'include!("../model/workspace.rs");']) {
      const declaration=file==='src/domain/workspace.rs'?'pub struct Workspace {}\n':'';
      assert.ok(violations({[file]:declaration+source},policy()).length>0);
    }
    const p=policy();p.exceptions[file]={dependencies:['lgui'],remove_in:'R8',reason:'bad'};
    const declaration=file==='src/domain/workspace.rs'?'pub struct Workspace {}\n':'';
    assert.ok(violations({[file]:declaration+'use lgui::prelude::*;'},p).some(e=>e.includes('domain cannot have migration exceptions')));
  }
});
test('workspace fields cannot evade their ownership register',()=>{
  const p=policy();p.workspace_core_ownership={gone:{owner:'domain/workspace',migrated_in:'R4'}};
  const errors=violations({'src/domain/workspace.rs':'pub struct Workspace<D, K> { pub docs: D, keys: K }'},p);
  assert.ok(errors.some(e=>e.includes('Workspace.docs: no registered owner')));
  assert.ok(errors.some(e=>e.includes('Workspace.gone: obsolete ownership entry')));
});
test('domain verification cannot silently include the application or an I/O adapter',()=>{
  const file='crates/workspace-tests/src/isolated.rs';
  for(const source of ['use lgui::ApplicationHandle;', '#[path = "../../../src/app/mod.rs"] mod app;', 'include!("../../../src/model/workspace.rs");']) {
    assert.ok(violations({[file]:source},policy()).length>0);
  }
  assert.deepEqual(violations({[file]:'#[path = "../../../src/domain/mod.rs"]\npub(crate) mod domain;'},policy()),[]);
});
test('domain verification cannot acquire GUI dependencies or renamed package aliases',()=>{
  const source='[dependencies]\nloom-editor-core = { path = "../editor-core" }\nserde = "1"\nserde_json = "1"\n';
  assert.deepEqual(verificationManifestErrors(source),[]);
  for(const modified of [source+'lgui = "1"\n',source+'[build-dependencies]\nlgui = "1"\n',source.replace('serde = "1"','serde = { package = "lgui", version = "1" }')])assert.ok(verificationManifestErrors(modified).length>0);
});

test('Editor views cannot regain root state, model, Vim machinery or exceptions',()=>{
  for(const file of ['src/ui/editor/view.rs','src/ui/editor/menu.rs','src/ui/editor_binding.rs']) {
    for(const source of ['use crate::app::state::AppState;', 'use crate::model::workspace::Workspace;', 'pub struct Scene { vim: Vim }', 'use crate::state::Root as Alias;', 'include!("../../app/state.rs");']) {
      assert.ok(violations({[file]:source},policy()).some(e=>e.includes('Editor view')));
    }
    const p=policy();p.exceptions[file]={dependencies:['crate::state'],reason:'bad',remove_in:'R8'};
    assert.ok(violations({[file]:'use crate::state::Root;'},p).some(e=>e.includes('cannot have migration exceptions')));
  }
});
test('nested editor fields still require ownership and cannot embed runtime services',()=>{
  const file='src/app/state/editor.rs',source='pub struct EditorState { pub loader: Option<FileLoader>, pub hover: Option<(u32, u32)>, }';
  assert.deepEqual(structFields(source,'EditorState'),['hover','loader']);
  const errors=violations({[file]:source},policy());
  assert.ok(errors.some(e=>e.includes('EditorState.loader: no registered owner')));
  assert.ok(errors.some(e=>e.includes('runtime service')));
  const p=policy();p.editor_state_ownership={gone:{owner:'app/state/editor',migrated_in:'R3'}};
  assert.ok(violations({[file]:'pub struct EditorState {}'},p).some(e=>e.includes('obsolete ownership')));
});

test('new reverse dependencies fail, including grouped imports',()=>{
  for(const source of ['use crate::ui::toast;', 'use crate::{app, ui::{toast}};', 'use lgui::prelude::*;','use std::fs;', 'use std::{fs, thread};', 'use lgui;', 'use unknown_external::Thing;']) {
    assert.ok(violations({'src/domain/new.rs':source},policy()).length>0);
  }
});
test('comments and strings do not masquerade as imports',()=>{
  assert.deepEqual(dependencies('// crate::app\n/* nested /* crate::ui */ comment */ const S: &str = r###"crate::state"###; let s = "lgui::Fake"; use crate::domain::notification;'),['crate::domain']);
});
test('existing exception does not authorize another source file',()=>{
  const p=policy();p.exceptions['src/ui/old.rs']={dependencies:['crate::state'],reason:'legacy',remove_in:'R8'};
  assert.ok(violations({'src/ui/old.rs':'use crate::state::A;','src/ui/new.rs':'use crate::state::A;'},p).some(e=>e.includes('new.rs')));
});
test('new root fields and runtime service fields fail',()=>{
  const source='pub struct AppState {\n pub loader: Option<FileLoader>,\n}\nimpl AppState {}';
  assert.deepEqual(stateFields(source),['loader']);
  const errors=violations({'src/app/state.rs':source},policy());
  assert.ok(errors.some(e=>e.includes('no registered owner')));
  assert.ok(errors.some(e=>e.includes('runtime service')));
});
test('new root view adapters and runtime bridges fail',()=>{
  assert.ok(violations({'src/ui/new.rs':'fn render(s: State<AppState>) { s.runtime(); }'},policy()).length>=2);
});
test('nested search snapshots cannot regain a runtime cancellation handle',()=>{
  const p=policy();p.exceptions['src/search_actions.rs']={dependencies:[],reason:'legacy',remove_in:'R2'};
  assert.ok(violations({'src/search_actions.rs':'pub struct SearchUiState { pub cancel: Option<Arc<AtomicBool>>, }\nimpl SearchUiState {}'},p).some(e=>e.includes('SearchState embeds')));
});
test('obsolete exceptions fail so debt cannot silently persist',()=>{
  const p=policy();p.exceptions['src/ui/old.rs']={dependencies:['crate::state'],reason:'legacy',remove_in:'R8'};
  assert.ok(violations({'src/ui/old.rs':'use crate::domain::notification;'},p).some(e=>e.includes('unused exception')));
});

test('Search feature and view cannot regain root or framework dependencies',()=>{
  for(const file of ['src/features/search/model.rs','src/ui/search/view.rs']) {
    assert.ok(violations({[file]:'use crate::app::state::AppState;'},policy()).length>0);
  }
  assert.ok(violations({'src/features/search/service.rs':'use std::fs;'},policy()).length>0);
});
test('Search does not accept legacy exceptions',()=>{
  const p=policy();p.exceptions['src/features/search/model.rs']={dependencies:['crate::state'],reason:'bad',remove_in:'R8'};
  assert.ok(violations({'src/features/search/model.rs':'use crate::state::Root;'},p).some(e=>e.includes('cannot have migration exceptions')));
});
test('Workspace cannot regain Search-specific snapshot contracts',()=>{
  const p=policy();p.exceptions['src/model/workspace.rs']={dependencies:['crate::features'],reason:'bad',remove_in:'R8'};
  assert.ok(violations({'src/model/workspace.rs':'use crate::features::search::BufferSnapshot;'},p).some(e=>e.includes('generic snapshots')));
});

test('the independent editing core rejects framework, application and I/O dependencies',()=>{
  for(const source of ['use lgui::core::*;', 'use crate::app::state::AppState;', 'use crate::model::workspace::Workspace;', 'use std::{fs, thread};', 'use std::process;', 'use conpty::Process;', 'use unknown_dependency::Thing;']) {
    assert.ok(violations({'crates/editor-core/src/new.rs':source},policy()).length>0);
  }
  assert.deepEqual(violations({'crates/editor-core/src/normal.rs':'use crate::text::EditorMut; use unicode_segmentation::UnicodeSegmentation;'},policy()),[]);
  assert.deepEqual(violations({'src/domain/identity.rs':'pub use loom_editor_core::identity::{FileId, PaneId};'},policy()),[]);
  assert.deepEqual(violations({'crates/editor-core/tests/host.rs':'use loom_editor_core::text::TextBuffer;'},policy()),[]);
});
test('the independent editing core cannot bypass its boundary with exceptions or source inclusion',()=>{
  const file='crates/editor-core/src/escape.rs',p=policy();
  p.exceptions[file]={dependencies:['lgui'],reason:'bad',remove_in:'R8'};
  assert.ok(violations({[file]:'use lgui::ApplicationHandle;'},p).some(e=>e.includes('cannot have migration exceptions')));
  for(const source of ['#[path = "../../../src/app/state.rs"] mod root;', 'include!("../../../src/app/state.rs");']) {
    assert.ok(violations({[file]:source},policy()).some(e=>e.includes('crate boundary')));
  }
});
test('Explorer rules cannot regain root state, GUI, I/O or exceptions',()=>{
  for(const source of ['use crate::state::AppState;','use lgui::prelude::*;','use std::fs;','use std::{process, thread};','use crate::infrastructure::explorer_filesystem::ExplorerFilesystem;','use crate::input::InputState;','pub struct Rows { input: InputState }','fn row(state: State<Tree>) {}']) {
    assert.ok(violations({'src/features/explorer/tree.rs':source},policy()).length>0,source);
  }
  assert.deepEqual(violations({'src/features/explorer/tree.rs':'use crate::features::explorer::DirEntry;\nuse std::path::Path;'},policy()),[]);
  const p=policy();p.exceptions['src/features/explorer/edit.rs']={dependencies:['crate::state'],reason:'bad',remove_in:'R8'};
  assert.ok(violations({'src/features/explorer/edit.rs':'use crate::state::AppState;'},p).some(e=>e.includes('Explorer feature cannot have migration exceptions')));
});
test('Explorer filesystem adapter is limited to its platform services',()=>{
  const file='src/infrastructure/explorer_filesystem.rs';
  assert.deepEqual(violations({[file]:'use crate::features::explorer::DirEntry;\nuse std::fs;\nfn t(){ std::process::Command::new("x"); windows_sys::Win32::Foo; }'},policy()),[]);
  for(const source of ['use crate::state::AppState;','use lgui::prelude::*;','use std::thread;','use crate::app::explorer;'])assert.ok(violations({[file]:source},policy()).length>0,source);
});
