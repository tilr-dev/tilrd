// Fails when code the library runs builds code at run time: `eval(` or `Function(`
// (`new Function(`) in the library's own modules, its generated code or any npm package
// its module graph reaches. A page with a strict Content-Security-Policy (no
// `unsafe-eval`) cannot run such code. Run by `scripts/sdk-check.sh`:
//
//   deno run --allow-run --allow-read tools/check_no_eval.ts
//
// Exits 1 naming each `file:line`.

const BUILDS_CODE = /(?<![\w$.])(?:new\s+)?Function\s*\(|(?<![\w$.])eval\s*\(/;
const SOURCE = /\.(?:[cm]?js|ts)$/;

interface Info {
  modules: { kind?: string; local?: string; npmPackage?: string }[];
  npmPackages?: Record<string, { localPath?: string; dependencies?: string[] }>;
}

function info(): Info {
  const out = new Deno.Command(Deno.execPath(), {
    args: ["info", "--frozen", "--json", "mod.ts"],
    stdout: "piped",
    stderr: "inherit",
  }).outputSync();
  if (!out.success) Deno.exit(2);
  return JSON.parse(new TextDecoder().decode(out.stdout));
}

function* files(dir: string): Generator<string> {
  for (const entry of Deno.readDirSync(dir)) {
    const path = `${dir}/${entry.name}`;
    if (entry.isDirectory) yield* files(path);
    else if (entry.isFile && SOURCE.test(entry.name) && !entry.name.endsWith(".d.ts")) yield path;
  }
}

const graph = info();
const scanned = new Set<string>();
for (const module of graph.modules) if (module.local) scanned.add(module.local);
// The npm packages the graph reaches, with what they depend on.
const packages = new Set<string>();
const pending = graph.modules.flatMap((module) => module.npmPackage ? [module.npmPackage] : []);
for (let name = pending.pop(); name; name = pending.pop()) {
  if (packages.has(name)) continue;
  packages.add(name);
  const found = graph.npmPackages?.[name];
  if (!found?.localPath) {
    console.error(`check_no_eval: ${name} is not in the cache (run deno check first)`);
    Deno.exit(2);
  }
  for (const path of files(found.localPath)) scanned.add(path);
  pending.push(...(found.dependencies ?? []));
}

let found = 0;
for (const path of [...scanned].sort()) {
  const lines = Deno.readTextFileSync(path).split("\n");
  lines.forEach((line, i) => {
    if (BUILDS_CODE.test(line)) {
      console.error(`${path}:${i + 1}: builds code at run time: ${line.trim().slice(0, 120)}`);
      found++;
    }
  });
}
if (found > 0) {
  console.error(`check_no_eval: ${found} place(s) build code at run time (eval, Function)`);
  Deno.exit(1);
}
console.log(`check_no_eval: ${scanned.size} files of mod.ts and ${packages.size} npm packages`);
