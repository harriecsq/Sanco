// Fixes a Qwik City 1.20.2 router bug that hits static (SSG) builds like ours.
// Runs on `npm install` (postinstall) and is safe to run again.
//
// On navigation the router replaces the destination with the `href` from the
// route's q-data.json. A prerendered q-data.json has no query string in it,
// and the router only puts the requested `?…` back when the previous URL had
// the very same query string. So going from `?page=2` to `?page=3`, or from a
// filtered list (`?q=x`) to `?id=…`, lost the query: the pager jumped back to
// page 1 and clicking a row in a filtered list just cleared the filters.
// The fix keeps the requested query whenever the router lands on the
// requested pathname (no redirect happened).

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const BROKEN =
  "if (navigation.dest.search && !!isSamePath(trackUrl, prevUrl)) {";
const FIXED =
  "if (navigation.dest.search && isSamePathname(trackUrl, navigation.dest)) {";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const lib = join(root, "node_modules", "@builder.io", "qwik-city");

let failed = false;
for (const name of ["index.qwik.mjs", "index.qwik.cjs"]) {
  const file = join(lib, "lib", name);
  const src = readFileSync(file, "utf8");
  if (src.includes(FIXED)) continue;
  if (!src.includes(BROKEN)) {
    console.error(
      `patch-qwik-city: ${file} doesn't contain the expected code. ` +
        "Qwik City was upgraded: check whether the query-string bug is fixed " +
        "and update or remove scripts/patch-qwik-city.mjs.",
    );
    failed = true;
    continue;
  }
  writeFileSync(file, src.replace(BROKEN, FIXED));
  console.log(`patch-qwik-city: patched ${name}`);
}
if (failed) process.exit(1);
