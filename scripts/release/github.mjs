// Bounded retries for transient transport/API failures, not workflow polling.
import {spawnSync} from 'node:child_process';
import {setTimeout as delay} from 'node:timers/promises';
export async function github(args, {runner = spawnSync, wait = delay, attempts = 3} = {}) {
  const timeout = args[0] === 'api' || args[1] === 'view' ? 30000 : 180000;
  for (let attempt = 0; attempt < attempts; attempt++) {
    let result;
    try {result = await runner('gh',args,{encoding:'utf8',stdio:['ignore','pipe','pipe'],timeout,windowsHide:true});}
    catch (error) {result = {status:1,error,stderr:error.message};}
    if (!result.error && result.status === 0) return result;
    const reason = result.stderr || result.error?.message || result.stdout || '';
    const transient = /HTTP (?:429|5\d\d)|connection reset|TLS handshake timeout|timed? out|temporary failure|unexpected EOF/i.test(reason) ||
      ['ETIMEDOUT','ECONNRESET','EAI_AGAIN'].includes(result.error?.code);
    if (/HTTP (?:401|403)|permission denied|bad credentials|authentication/i.test(reason) || !transient || attempt + 1 === attempts) {
      if (result.error) throw result.error;
      return result;
    }
    await wait(1000 * 2 ** attempt);
  }
}
