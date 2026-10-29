// Verified, atomic downloads with bounded retries for transient network failures.
import fs from 'node:fs';
import path from 'node:path';
import {createHash, randomUUID} from 'node:crypto';
import {Readable} from 'node:stream';
import {pipeline} from 'node:stream/promises';
import {setTimeout as delay} from 'node:timers/promises';

export async function sha256(file) {
  const hash = createHash('sha256');
  for await (const chunk of fs.createReadStream(file)) hash.update(chunk);
  return hash.digest('hex');
}
const networkFailure = error => error?.name === 'TimeoutError' || error instanceof TypeError ||
  /^(ECONNRESET|ECONNREFUSED|ETIMEDOUT|EAI_AGAIN|ENETUNREACH|EHOSTUNREACH|EPIPE|UND_ERR_)/.test(error?.code || '') ||
  (error?.cause && networkFailure(error.cause));
export function retryAfter(value, now = Date.now()) {
  if (!value) return 0;
  const seconds = Number(value);
  return Number.isFinite(seconds) ? Math.max(0, seconds * 1000) : Math.max(0, Date.parse(value) - now) || 0;
}

export async function downloadVerified({url, destination, sha256: expected},
  {fetcher = fetch, attempts = 4, timeoutMs = 300000, wait = delay, now = Date.now} = {}) {
  if (!Number.isInteger(attempts) || attempts < 1 || !(timeoutMs > 0)) throw Error('Invalid download retry budget');
  fs.mkdirSync(path.dirname(destination), {recursive:true});
  const deadline = now() + timeoutMs;
  for (let attempt = 0; attempt < attempts; attempt++) {
    const remaining = deadline - now();
    if (remaining <= 0) throw Error('Download time budget exhausted: ' + url);
    const temporary = destination + '.' + randomUUID() + '.partial';
    let retry = false, serverDelay = 0, failure;
    try {
      let response;
      try {response = await fetcher(url, {redirect:'follow', signal:AbortSignal.timeout(Math.max(1, Math.ceil(remaining)))});}
      catch (error) {retry = Boolean(networkFailure(error)); throw error;}
      if (!response.ok || !response.body) {
        retry = response.status === 408 || response.status === 429 || response.status >= 500;
        serverDelay = retryAfter(response.headers.get('retry-after'), now());
        await response.body?.cancel().catch(() => {});
        throw Error('Download failed (' + response.status + '): ' + url);
      }
      try {await pipeline(Readable.fromWeb(response.body), fs.createWriteStream(temporary, {flags:'wx'}));}
      catch (error) {retry = Boolean(networkFailure(error)); throw error;}
      if (await sha256(temporary) !== expected) throw Error('SHA-256 mismatch: ' + url);
      fs.renameSync(temporary, destination);
      return destination;
    } catch (error) {failure = error;}
    finally {fs.rmSync(temporary, {force:true});}
    const pause = Math.max(1000 * 2 ** attempt, serverDelay);
    if (!retry || attempt + 1 === attempts || pause >= deadline - now()) throw failure;
    await wait(pause);
  }
}
