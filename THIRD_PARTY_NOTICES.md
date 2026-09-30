# Third-party notices

Official Loom packages may include a managed Git runtime. Git is licensed under
the GNU General Public License version 2. Runtime packages must retain Git's
license, source-offer information, upstream version, download URL, and the
generated `runtime/git/MANIFEST.json` file.

The packaging pipeline treats the runtime as a separate executable component;
Loom invokes it through its command-line interface and does not link Git into
the Loom binary.
