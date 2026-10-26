import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {spawn, spawnSync} from 'node:child_process';
import {setTimeout as delay} from 'node:timers/promises';
import {parseArgs} from 'node:util';
import {fileURLToPath} from 'node:url';
import {ROOT} from '../dependencies/ensure.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
export function normalizeWindowsPath(value) {
  return value.replace(/^\\\\\?\\UNC\\/i, '\\\\').replace(/^\\\\\?\\/, '').replaceAll('/', '\\').replace(/\\$/, '').toLowerCase();
}
export function buildCloseHelper(directory) {
  const executable = path.join(directory, 'close-window.exe');
  const result = spawnSync('rustc', ['--edition=2024', '--crate-name=loom_baseline_close_window',
    path.join(HERE, 'close-window.rs'), '-o', executable], {cwd: ROOT, encoding: 'utf8', windowsHide: true, timeout: 60000});
  fs.writeFileSync(path.join(directory, 'close-helper-build.log'), (result.stdout || '') + (result.stderr || '') + (result.error?.message || ''));
  if (result.error || result.status !== 0) throw Error(`Could not compile window close helper: ${result.error?.message || result.stderr}`);
  return executable;
}

export async function startupSmoke({executable = path.join(ROOT, 'target/release/Loom.exe'), outputDirectory,
  startupMs = 4000, closeTimeoutMs = 10000, env = process.env, signal} = {}) {
  if (process.platform !== 'win32') throw Error('This startup smoke requires Windows');
  if (!outputDirectory) throw Error('An output directory is required');
  executable = fs.realpathSync(executable);
  outputDirectory = path.resolve(outputDirectory);
  if (fs.existsSync(outputDirectory) && fs.readdirSync(outputDirectory).length) throw Error('Choose a fresh, empty startup smoke output directory');
  const fixture = path.join(outputDirectory, 'startup-fixture');
  const appdata = path.join(fixture, 'appdata'), localdata = path.join(fixture, 'localappdata'), workspace = path.join(fixture, 'workspace');
  for (const directory of [appdata, localdata, workspace]) fs.mkdirSync(directory, {recursive: true});
  fs.writeFileSync(path.join(workspace, '示例.txt'), 'Loom startup smoke: 中文 + emoji 🙂\n');
  const result = {exe: executable, sha256: createHash('sha256').update(fs.readFileSync(executable)).digest('hex'),
    child_appdata: appdata, child_localappdata: localdata, windows_hide: true, started_at: new Date().toISOString(),
    result: 'failed', forced_cleanup: false};
  let child, finished, descriptor, exited = false, spawnError;
  const abort = () => { if (child && !exited) {result.forced_cleanup = child.kill();} };
  try {
    const helper = buildCloseHelper(outputDirectory);
    if (signal?.aborted) throw Error('Startup smoke cancelled');
    descriptor = fs.openSync(path.join(outputDirectory, 'startup-process.log'), 'w');
    child = spawn(executable, [workspace], {cwd: workspace, env: {...env, APPDATA: appdata, LOCALAPPDATA: localdata},
      stdio: ['ignore', descriptor, descriptor], windowsHide: true});
    result.pid = child.pid;
    finished = new Promise(resolve => {
      child.on('error', error => {spawnError = error;});
      child.on('close', (code, termination) => {exited = true; result.exit_code = code; result.signal = termination; resolve();});
    });
    signal?.addEventListener('abort', abort, {once: true});
    if (signal?.aborted) abort();
    // Race the observation timer with exit; clear it when a child dies early.
    const observation = new AbortController();
    await Promise.race([finished, delay(startupMs, undefined, {signal: observation.signal}).catch(() => {})]);
    observation.abort();
    if (signal?.aborted) throw Error('Startup smoke cancelled');
    if (spawnError) throw spawnError;
    result.alive_after_4_seconds = !exited;
    result.observation_milliseconds = startupMs;
    result.root_effect_configuration_files = fs.readdirSync(appdata, {recursive: true}).filter(name => fs.statSync(path.join(appdata, name)).isFile());
    if (exited) throw Error(`Loom exited during startup, code ${result.exit_code}`);
    const request = spawnSync(helper, [String(child.pid), executable], {encoding: 'utf8', windowsHide: true, timeout: 5000});
    result.close_requested = !request.error && request.status === 0;
    if (!result.close_requested) throw Error(`Could not request normal window close: ${request.error?.message || request.stderr}`);
    const timeout = new AbortController();
    await Promise.race([finished, delay(closeTimeoutMs, undefined, {signal: timeout.signal}).catch(() => {})]);
    timeout.abort();
    if (signal?.aborted) throw Error('Startup smoke cancelled');
    if (!exited) throw Error('The isolated Loom process did not close normally');
    result.clean_exit = result.exit_code === 0;
    if (!result.clean_exit) throw Error(`Loom exited with code ${result.exit_code}`);
    const session = path.join(appdata, 'Loom/workspace.json');
    if (!fs.existsSync(session)) throw Error('Loom did not save an isolated workspace session');
    const saved = JSON.parse(fs.readFileSync(session, 'utf8').replace(/^\uFEFF/, ''));
    if (!Array.isArray(saved.open_folders) || saved.open_folders.length !== 1 ||
      typeof saved.open_folders[0] !== 'string' || normalizeWindowsPath(saved.open_folders[0]) !== normalizeWindowsPath(workspace)) {
      throw Error('The saved session does not identify the synthetic workspace');
    }
    result.saved_synthetic_workspace = true;
    result.result = 'passed';
  } catch (error) {result.error = error.message;}
  finally {
    signal?.removeEventListener('abort', abort);
    if (child && !exited) {
      result.forced_cleanup = child.kill() || result.forced_cleanup;
      const cleanup = new AbortController();
      await Promise.race([finished, delay(5000, undefined, {signal: cleanup.signal}).catch(() => {})]);
      cleanup.abort();
      if (!exited) result.cleanup_error = 'Smoke child did not exit after forced cleanup';
    }
    if (descriptor !== undefined) fs.closeSync(descriptor);
    result.finished_at = new Date().toISOString();
    fs.writeFileSync(path.join(outputDirectory, 'startup-smoke.json'), JSON.stringify(result, null, 2) + '\n');
  }
  return result;
}

if (path.resolve(process.argv[1] || '') === fileURLToPath(import.meta.url)) {
  const controller = new AbortController(), abort = () => controller.abort();
  process.once('SIGINT', abort);
  process.once('SIGTERM', abort);
  try {
    const {values} = parseArgs({options: {help: {type: 'boolean'}, executable: {type: 'string'}, 'output-directory': {type: 'string'}}});
    if (values.help) console.log('Usage: node scripts/baseline/startup-smoke.mjs --output-directory <fresh-directory> [--executable <Loom.exe>]');
    else {
      const result = await startupSmoke({executable: values.executable, outputDirectory: values['output-directory'], signal: controller.signal});
      console.log(`Startup smoke ${result.result}: ${result.error || values['output-directory']}`);
      process.exitCode = result.result === 'passed' ? 0 : 1;
    }
  } catch (error) {console.error(error.message); process.exitCode = 1;}
  finally {process.removeListener('SIGINT', abort); process.removeListener('SIGTERM', abort);}
}
