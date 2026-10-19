import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {archive, extract} from './cache.mjs';

export function cmakeSpec(manifest, platform, arch) {
  const spec = manifest.cmake.hosts[`${platform}-${arch}`];
  if (!spec) throw Error(`Unsupported CMake host: ${platform}-${arch}`);
  return spec;
}

export async function prepareCmake(manifest, options) {
  const spec = cmakeSpec(manifest, options.platform, options.arch);
  const file = await archive(spec, options);
  const directory = await extract(spec, file, options);
  const executable = path.join(directory, spec.executable);
  const version = execFileSync(executable, ['--version'], {encoding: 'utf8'});
  if (!version.startsWith(`cmake version ${manifest.cmake.version}\n`) && !version.startsWith(`cmake version ${manifest.cmake.version}\r\n`)) throw Error(`Unexpected CMake version: ${version}`);
  return {executable, bin: path.dirname(executable), archive: file};
}
