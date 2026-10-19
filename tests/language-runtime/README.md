# Independent language runtime verification

This Cargo workspace compiles production domain, syntax, loader and worker
sources directly. It has no concrete grammar crate, Rust wrapper or language
build project dependency. Use standard upstream native modules.
Supply a complete prebuilt collection (the regression fixtures expect the eight
bundled packages); this suite never builds grammar modules:

~~~sh
node run.mjs --packages /path/to/prebuilt-packages
~~~

Use --offline for cached Rust dependencies, or --probe-only to run just the
production-mode install/restart probe. The probe verifies plain text without
resources, JSON/Markdown/custom-extension highlighting after portable and user installation,
an unchanged executable hash,
no implicit Rust activation, and disabling on restart. It runs in a temporary
location with isolated user data variables and removes its files after exit.

The host's default cargo test does not enable these parser tests and requires no
language artifacts. Host metadata unit fixtures contain no parser/query payload.

To additionally verify a genuinely new grammar outside the baseline collection,
supply any complete external package and a sample whose extension it registers:

~~~sh
node run.mjs --packages /path/to/baseline --new-package /path/to/python --sample /path/to/new.py --probe-only
~~~

The same production-mode executable must show the sample as plain text before
installation, then recognize and highlight it after copying only those resource
files into the user directory. No host source, dependency or binary changes.
