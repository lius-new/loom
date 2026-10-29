import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {parseArgs} from 'node:util';
import {parseReleaseNotes} from '../release/notes.mjs';

const guidance = 'Every commit needs Release Notes:\n- N/A\nFor user-facing changes, replace N/A with English Added/Improved/Changed/Removed/Fixed/Upgrade bullets.';
export function checkMessage(message) {
  const lines = message.replaceAll('\r\n', '\n').split('\n');
  const headings = [];
  let fence;
  for (const [index, line] of lines.entries()) {
    const marker = line.match(/^\s*(\x60{3,}|~{3,})/);
    if (marker) {
      if (!fence) fence = {char: marker[1][0], length: marker[1].length};
      else if (marker[1][0] === fence.char && marker[1].length >= fence.length) fence = undefined;
    } else if (!fence && /^(?:#{1,6}\s+)?Release Notes:?\s*$/i.test(line)) headings.push(index);
  }
  if (headings.length !== 1 || lines[headings[0]].trimEnd() !== 'Release Notes:') {
    throw new Error('Use exactly one Release Notes: heading outside code fences.\n' + guidance);
  }
  const parsed = parseReleaseNotes(message);
  if (!parsed.present) throw new Error('Release Notes section is not readable by the release generator.');
  const section = [];
  for (const line of lines.slice(headings[0] + 1)) {
    if (/^(?:#{1,6}\s|[A-Za-z][A-Za-z-]*:\s|\s*-{3,}\s*$)/.test(line)) break;
    section.push(line);
  }
  if (section.some(line => /^[-*]\s+N\/A[.!]?\s*$/i.test(line)) &&
      (parsed.entries.length || section.filter(line => /^[-*]\s+/.test(line)).length !== 1)) {
    throw new Error('Use a single - N/A for internal changes, or meaningful release notes; do not mix them.');
  }
  return parsed;
}

function git(cwd, args) {
  return execFileSync('git', args, {cwd, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024, stdio: ['pipe', 'pipe', 'pipe']}).trim();
}
const sha = value => typeof value === 'string' && /^[a-f0-9]{40}(?:[a-f0-9]{24})?$/i.test(value) && !/^0+$/.test(value);
export function checkEvent({eventName, event, cwd = process.cwd()}) {
  let revision;
  if (eventName === 'workflow_dispatch') revision = git(cwd, ['rev-parse', '--verify', 'HEAD']);
  else if (eventName === 'push' || eventName === 'pull_request') {
    const base = eventName === 'push' ? event.before : event.pull_request?.base?.sha;
    const head = eventName === 'push' ? event.after : event.pull_request?.head?.sha;
    if (!sha(head)) throw new Error('Commit check has no valid event head SHA.');
    // A newly created branch has no previous tip. Check its tip, not legacy history.
    if (eventName === 'push' && typeof base === 'string' && /^0{40}(?:0{24})?$/.test(base)) revision = head;
    else {
      if (!sha(base)) throw new Error('Commit check has no valid event base SHA.');
      revision = base + '..' + head;
    }
  } else throw new Error('Unsupported commit-check event: ' + eventName);
  const args = ['log', '--reverse', '--format=%H%x00%B%x00'];
  if (!revision.includes('..')) args.push('-1');
  const fields = git(cwd, [...args, revision, '--']).split('\0');
  const failures = [];
  let count = 0;
  for (let i = 0; i + 1 < fields.length; i += 2) {
    const hash = fields[i].trim();
    count++;
    try { checkMessage(fields[i + 1]); }
    catch (error) { failures.push(hash.slice(0, 12) + ': ' + error.message); }
  }
  if (!count) throw new Error('No commits found in event range; refusing to skip commit validation.');
  if (failures.length) throw new Error(failures.join('\n\n'));
  return count;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const {values} = parseArgs({options: {'message-file': {type: 'string'}, event: {type: 'boolean'}}});
    if (Boolean(values['message-file']) === Boolean(values.event)) throw new Error('Choose --message-file <path> or --event.');
    if (values.event) {
      const count = checkEvent({eventName: process.env.GITHUB_EVENT_NAME, event: JSON.parse(fs.readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'))});
      console.log('Release Notes validated for ' + count + ' commit(s).');
    } else {
      // Match Git's comment handling for editor templates, including core.commentChar.
      const message = execFileSync('git', ['stripspace', '--strip-comments'], {input: fs.readFileSync(values['message-file']), encoding: 'utf8'});
      checkMessage(message);
    }
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
