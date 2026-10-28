// The engine bundled by release-please-action v5.0.0, with run-local baseline overrides.
const {execFileSync} = require('node:child_process');
const {appendFileSync} = require('node:fs');
const {Manifest, GitHub} = require('release-please');
const {Version} = require('release-please/build/src/version');

function git(repo, ...args) {
  return execFileSync('git', args, {cwd: repo, encoding: 'utf8'}).trim();
}

function numbers(value) {
  return value.split('.').map(Number);
}

function compare(a, b) {
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
  }
  return 0;
}

function planRelease(repo, branch, bootstrap) {
  if (!/^release\/(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(branch)) {
    throw new Error(`release branches are named release/X.Y, not ${branch}`);
  }
  const line = branch.slice('release/'.length);
  const target = numbers(`${line}.0`);
  if (!target.every(Number.isSafeInteger)) throw new Error('Release line is too large');
  const stable = git(repo, 'tag', '--list', 'v*').split('\n')
    .filter(tag => /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(tag))
    .map(tag => ({tag, version: tag.slice(1), parts: numbers(tag.slice(1))}))
    .filter(tag => tag.parts.every(Number.isSafeInteger))
    .sort((a, b) => compare(b.parts, a.parts));
  const sameLine = stable.find(tag => compare(tag.parts.slice(0, 2), target.slice(0, 2)) === 0);
  const baseline = sameLine || stable.find(tag => compare(tag.parts, target) < 0);
  let boundary = bootstrap;
  let included;
  if (baseline) {
    const sha = git(repo, 'rev-parse', `${baseline.tag}^{commit}`);
    const bases = git(repo, 'merge-base', '--all', 'HEAD', sha).split('\n');
    if (bases.length !== 1 || !bases[0]) throw new Error('No unique shared release history');
    boundary = bases[0];
    if (sameLine && boundary !== sha) {
      throw new Error(`Stable ${baseline.tag} is outside this release branch history`);
    }
    // git cherry compares patch IDs, so a fix cherry-picked onto the old release
    // branch is excluded even though main has a different SHA for the same patch.
    included = new Set(git(repo, 'rev-list', 'HEAD', `^${sha}`).split('\n').filter(Boolean));
    for (const row of git(repo, 'cherry', sha, 'HEAD').split('\n')) {
      if (row.startsWith('- ')) included.delete(row.slice(2));
    }
  }
  if (!boundary) throw new Error('No stable release or configured bootstrap SHA');
  git(repo, 'merge-base', '--is-ancestor', boundary, 'HEAD');
  const patch = sameLine ? sameLine.parts[2] + 1 : 0;
  if (!Number.isSafeInteger(patch)) throw new Error('Release patch is too large');
  return {
    version: `${line}.${patch}`, baseline, boundary, included,
    known: new Set(git(repo, 'rev-list', 'HEAD').split('\n')),
    // Include the boundary itself; do not silently truncate a long release history.
    depth: Number(git(repo, 'rev-list', '--count', `${boundary}..HEAD`)) + 1,
  };
}

async function releaseManifest(github, branch, repo) {
  const config = await github.getFileJson('release-please-config.json', branch);
  const plan = planRelease(repo, branch, config['bootstrap-sha']);
  const filtered = new Proxy(github, {
    get(target, key) {
      if (key === 'mergeCommitIterator') {
        return async function* (...args) {
          for await (const commit of target.mergeCommitIterator(...args)) {
            if (!plan.known.has(commit.sha)) {
              throw new Error('Release branch changed since checkout; run the workflow at its current head');
            }
            if (commit.sha === plan.boundary || !plan.included || plan.included.has(commit.sha)) {
              yield commit;
            }
          }
        };
      }
      const value = Reflect.get(target, key);
      return typeof value === 'function' ? value.bind(target) : value;
    },
  });
  const manifest = await Manifest.fromManifest(filtered, branch, undefined, undefined, {
    lastReleaseSha: plan.boundary,
    commitSearchDepth: plan.depth,
  });
  for (const path of Object.keys(manifest.repositoryConfig)) {
    manifest.repositoryConfig[path].releaseAs = plan.version;
    if (plan.baseline) manifest.releasedVersions[path] = Version.parse(plan.baseline.version);
  }
  return {manifest, plan};
}

function releaseOutputs(releases, paths) {
  const outputs = {releases_created: releases.length > 0};
  for (const path of paths) outputs[path === '.' ? 'release_created' : `${path}--release_created`] = false;
  for (const release of releases) {
    const prefix = release.path === '.' ? '' : `${release.path}--`;
    outputs[`${prefix}release_created`] = true;
    outputs[`${prefix}tag_name`] = release.tagName;
  }
  return outputs;
}

async function run(github, branch, repo, output) {
  // Use the checked-in manifest to recognize merged release PRs. The next-version
  // override belongs only to PR construction, never to tagging the merged PR.
  const head = await github.mergeCommitIterator(branch, {maxResults: 1}).next();
  if (head.done || head.value.sha !== git(repo, 'rev-parse', 'HEAD')) {
    throw new Error('Release branch changed since checkout; run the workflow at its current head');
  }
  const original = await Manifest.fromManifest(github, branch);
  // Validate history before any writes, including publication of a merged PR.
  const prepared = await releaseManifest(github, branch, repo);
  const releases = (await original.createReleases()).filter(Boolean);
  for (const [key, value] of Object.entries(releaseOutputs(releases, Object.keys(original.repositoryConfig)))) {
    output(key, value);
  }
  // A newly published release has no subsequent commits in this push. A later push
  // or re-run checks out its tags and constructs the next patch PR if needed.
  if (!releases.length) {
    const {manifest, plan} = prepared;
    console.log(`Release ${plan.version}; baseline ${plan.baseline?.tag || 'bootstrap'}; boundary ${plan.boundary}`);
    await manifest.createPullRequests();
  }
}

async function main() {
  const branch = process.env.BRANCH;
  if (!/^release\/(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(branch || '')) {
    throw new Error(`release branches are named release/X.Y, not ${branch}`);
  }
  const [owner, repo] = (process.env.GITHUB_REPOSITORY || '').split('/');
  if (!owner || !repo || !process.env.GH_TOKEN) throw new Error('Missing repository or GitHub token');
  const github = await GitHub.create({owner, repo, token: process.env.GH_TOKEN, defaultBranch: branch});
  await run(github, branch, process.cwd(), (key, value) => {
    appendFileSync(process.env.GITHUB_OUTPUT, `${key}=${value}\n`);
  });
}

module.exports = {planRelease, releaseManifest, releaseOutputs, run};
if (require.main === module) main().catch(error => {console.error(error); process.exitCode = 1;});
