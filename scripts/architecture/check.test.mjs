import test from 'node:test';
import assert from 'node:assert/strict';
import {dependencies,stateFields,violations} from './check.mjs';
const policy=()=>({exceptions:{},root_state_adapters:[],runtime_bridges:[],state_ownership:{}});

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
