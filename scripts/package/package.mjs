#!/usr/bin/env node
// Loom packaging entry point.
//
//   node scripts/package/package.mjs <step> [options]
//
// Steps run in this order; `all` runs every step:
//   setup    Install the packaging toolchain into .packaging/tools
//   compile  Build the release binary with Cargo
//   package  Stage the application folder and write the portable archive
//   bundle   Build the installer from the staged folder
//
// Options:
//   --platform <name>         windows (default on Windows), macos, linux
//   --expect-version <x.y.z>  Fail unless Cargo.toml has this version
//
// Downloads, tools, staging folders and final artifacts all live under the
// git-ignored `.packaging/` directory; nothing is installed system-wide.

import path from 'node:path';

import * as windows from './windows/index.mjs';
import { DIST_DIR, PackagingError, ROOT, cargoPackageField, fail } from './lib/util.mjs';

const STEPS = ['setup', 'compile', 'package', 'bundle'];

const PLATFORMS = {
  windows: {
    setup: windows.setup,
    compile: windows.compile,
    package: windows.packageApp,
    bundle: windows.bundle,
  },
};

const PLANNED_PLATFORMS = new Set(['macos', 'linux']);

function usage() {
  console.log(`Usage: node scripts/package/package.mjs <${[...STEPS, 'all'].join('|')}> [options]

Options:
  --platform <name>         windows (default on Windows), macos, linux
  --expect-version <x.y.z>  Fail unless Cargo.toml has this version
  -h, --help                Show this help`);
}

function parseArguments(argv) {
  const options = { step: undefined, platform: defaultPlatform(), expectVersion: undefined };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    switch (argument) {
      case '-h':
      case '--help':
        usage();
        process.exit(0);
        break;
      case '--platform':
      case '--expect-version': {
        const value = argv[index + 1];
        if (value === undefined || value.startsWith('--')) {
          fail(`${argument} needs a value`);
        }
        index += 1;
        if (argument === '--platform') {
          options.platform = value;
        } else {
          options.expectVersion = value.replace(/^v/, '');
        }
        break;
      }
      default:
        if (argument.startsWith('-') || options.step !== undefined) {
          fail(`Unexpected argument: ${argument}`);
        }
        options.step = argument;
    }
  }
  if (options.step === undefined) {
    usage();
    process.exit(1);
  }
  if (options.step !== 'all' && !STEPS.includes(options.step)) {
    fail(`Unknown step: ${options.step}`);
  }
  return options;
}

function defaultPlatform() {
  return { win32: 'windows', darwin: 'macos', linux: 'linux' }[process.platform];
}

async function main() {
  const [major] = process.versions.node.split('.').map(Number);
  if (major < 20) {
    fail(`Node.js 20 or newer is required (found ${process.versions.node}).`);
  }

  const options = parseArguments(process.argv.slice(2));
  const platform = PLATFORMS[options.platform];
  if (!platform) {
    if (PLANNED_PLATFORMS.has(options.platform)) {
      fail(`Packaging for ${options.platform} is not implemented yet.`);
    }
    fail(`Unknown platform: ${options.platform ?? process.platform}`);
  }

  const version = cargoPackageField('version');
  if (options.expectVersion !== undefined && options.expectVersion !== version) {
    fail(`Requested version ${options.expectVersion} does not match Cargo.toml version ${version}`);
  }

  const steps = options.step === 'all' ? STEPS : [options.step];
  for (const name of steps) {
    await platform[name]({ version });
  }
  if (steps.includes('package') || steps.includes('bundle')) {
    console.log(`\nArtifacts: ${path.relative(ROOT, DIST_DIR)}`);
  }
}

main().catch((error) => {
  if (error instanceof PackagingError) {
    console.error(`\nerror: ${error.message}`);
  } else {
    console.error(error);
  }
  process.exit(1);
});
