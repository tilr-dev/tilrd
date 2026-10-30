const {test} = require('node:test');
const assert = require('node:assert/strict');
const {readFileSync, existsSync} = require('node:fs');
const path = require('node:path');
const root=path.resolve(__dirname,'..');
const workflow=readFileSync(path.join(root,'.github/workflows/release.yml'),'utf8');
const ci=readFileSync(path.join(root,'.github/workflows/ci.yml'),'utf8');
function job(name) {return workflow.split(`\n  ${name}:\n`)[1].split(/\n  [a-z][\w-]*:\n/)[0];}
test('caller pins the shared implementation and keeps its repository-scoped bot credentials',()=>{
  const release=job('release');
  const [,sha]=release.match(/uses: tilr-dev\/\.github\/\.github\/workflows\/release-branch.yml@([a-f0-9]{40})\n/);
  assert.equal(sha, 'c31b80fcfd5cb9398a193d422c212326302d0c44');
  assert.ok(release.includes(`workflow-sha: ${sha}\n`));
  assert.ok(ci.includes(`ref: ${sha}\n`));
  assert.match(release,/contents: read/);
  assert.match(release,/app-client-id: \$\{\{ vars.TILR_BOT_CLIENT_ID \}\}/);
  assert.match(release,/app-private-key: \$\{\{ secrets.TILR_BOT_PRIVATE_KEY \}\}/);
  assert.match(workflow,/on:\n  push:\n    branches: \['release\/\*\*'\]/);
  assert.match(workflow,/cancel-in-progress: false/);
  assert.match(job('build'),/if: needs.release.outputs.tag != ''/);
  assert.match(job('build'),/ref: \$\{\{ needs.release.outputs.tag \}\}/);
  assert.match(job('upload'),/gh release upload/);
});
test('candidates retain beta CDN publication and stable releases retain stable publication',()=>{
  assert.doesNotMatch(job('release'),/package-path|release_created/);
  assert.match(job('cdn'),/if: needs.release.outputs.tag != ''/);
  assert.match(job('cdn'),/outputs.is_rc == 'false' && 'stable' \|\| 'beta'/);
  assert.match(job('cdn'),/scripts\/release.sh --upload-only --channel/);
});

test('releases propose no desktop pin: the app updates tilrd itself',()=>{
  assert.doesNotMatch(workflow,/bump-desktop-pin|desktop-mac|bot\/bump-tilrd/);
  assert.ok(!existsSync(path.join(root,'scripts/bump-desktop-pin.sh')));
});

test('stable tags trigger directly, RC tags are excluded, and publication is queued',()=>{
  assert.ok(workflow.includes("    tags: ['v*', '!v*-rc.*']\n"));
  assert.match(workflow,/queue: max/);
  assert.match(job('build'),/python3 scripts\/check-release-version.py \"\$TAG\"/);
});
