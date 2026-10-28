const {test} = require('node:test');
const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const path = require('node:path');
const root=path.resolve(__dirname,'..');
const workflow=readFileSync(path.join(root,'.github/workflows/release.yml'),'utf8');
const ci=readFileSync(path.join(root,'.github/workflows/ci.yml'),'utf8');
function job(name) {return workflow.split(`\n  ${name}:\n`)[1].split(/\n  [a-z][\w-]*:\n/)[0];}
test('caller pins the shared implementation and keeps its repository-scoped bot credentials',()=>{
  const release=job('release-please');
  const [,sha]=release.match(/uses: tilr-dev\/\.github\/\.github\/workflows\/release-branch.yml@([a-f0-9]{40})\n/);
  assert.ok(release.includes(`workflow-sha: ${sha}\n`));
  assert.ok(ci.includes(`ref: ${sha}\n`));
  assert.match(release,/contents: read/);
  assert.match(release,/app-client-id: \$\{\{ vars.TILR_BOT_CLIENT_ID \}\}/);
  assert.match(release,/app-private-key: \$\{\{ secrets.TILR_BOT_PRIVATE_KEY \}\}/);
  assert.match(workflow,/on:\n  push:\n    branches: \['release\/\*\*'\]/);
  assert.match(workflow,/cancel-in-progress: false/);
  assert.match(job('build'),/if: needs.release-please.outputs.tag != ''/);
  assert.match(job('build'),/ref: \$\{\{ needs.release-please.outputs.tag \}\}/);
  assert.match(job('upload'),/gh release upload/);
});
test('candidates retain beta CDN publication and stable releases retain stable publication',()=>{
  assert.match(job('release-please'),/package-path: '\.'/);
  assert.match(job('cdn'),/if: needs.release-please.outputs.tag != ''/);
  assert.match(job('cdn'),/outputs.release_created == 'true' && 'stable' \|\| 'beta'/);
  assert.match(job('cdn'),/scripts\/release.sh --upload-only --channel/);
});
