const {test} = require('node:test');
const assert = require('node:assert/strict');
const {execFileSync} = require('node:child_process');
const {mkdtempSync, writeFileSync, mkdirSync, rmSync, readFileSync} = require('node:fs');
const {tmpdir} = require('node:os');
const path = require('node:path');
const {Manifest} = require('release-please');
const {releaseManifest, planRelease, releaseOutputs, run} = require('./run.cjs');
const root = path.resolve(__dirname, '../..');

function fixture(t, version = '0.0.1') {
  const repo = mkdtempSync(path.join(tmpdir(), 'release-line-'));
  t.after(() => rmSync(repo, {recursive:true, force:true}));
  const git = (...args) => execFileSync('git', args, {cwd:repo, encoding:'utf8', stdio:['ignore','pipe','pipe']}).trim();
  git('init','-b','main'); git('config','user.name','Release Test'); git('config','user.email','test@example.invalid');
  git('config','commit.gpgsign','false'); git('config','tag.gpgsign','false');
  const put = (name, text) => {mkdirSync(path.dirname(path.join(repo,name)),{recursive:true});writeFileSync(path.join(repo,name),text);};
  const config = JSON.parse(readFileSync(path.join(root, 'release-please-config.json')));
  const packagePath = Object.keys(config.packages)[0];
  const file = packagePath === '.' ? 'new.txt' : `${packagePath}/src/new.rs`;
  put('release-please-config.json',JSON.stringify(config));
  put('.release-please-manifest.json',JSON.stringify({[packagePath]:version}));
  put('version.txt',version);
  if (packagePath !== '.') put(`${packagePath}/Cargo.toml`, `[package]\nname = "gitpm"\nversion = "${version}"\n`);
  let n=0;
  function commit(message, name=file, text=`value ${++n}\n`) {
    put(name,text); git('add','.'); git('commit','-m',message); return git('rev-parse','HEAD');
  }
  const base=commit('chore: initial'); git('tag',`v${version}`);
  config['bootstrap-sha']=base;
  put('release-please-config.json',JSON.stringify(config));
  commit('chore: configure bootstrap');
  const github = {
    repository:{owner:'test',repo:'release-lines'},
    async getFileJson(name, branch) {return JSON.parse(git('show',`${branch}:${name}`));},
    async getFileContentsOnBranch(name, branch) {
      const parsedContent=git('show',`${branch}:${name.replace(/^\//,'')}`);
      return {parsedContent,content:Buffer.from(parsedContent).toString('base64')};
    },
    async *releaseIterator() {
      for (const tag of git('tag','--sort=-version:refname').split('\n').filter(Boolean)) {
        yield {tagName:tag,sha:git('rev-parse',`${tag}^{commit}`)};
      }
    },
    async *mergeCommitIterator(branch) {
      for (const sha of git('rev-list',branch).split('\n')) {
        yield {sha,message:git('show','-s','--format=%B',sha),files:git('diff-tree','--root','--no-commit-id','--name-only','-r',sha).split('\n')};
      }
    },
    async *pullRequestIterator() {},
    async *tagIterator() {},
  };
  async function proposal(branch, fixed=true) {
    git('checkout',branch);
    const manifest=fixed ? (await releaseManifest(github,branch,repo)).manifest : await Manifest.fromManifest(github,branch);
    const prs=await manifest.buildPullRequests();
    return prs.map(pr => ({version:pr.version.toString(),body:pr.body.toString(),updates:pr.updates}));
  }
  return {repo,git,put,commit,base,github,proposal,packagePath,file};
}

test('a second release line excludes changes shipped on the other branch',async t=>{
  const f=fixture(t);
  f.commit('feat: already shipped');
  f.git('checkout','-b','release/0.1'); f.commit('chore: release 0.1.0'); f.git('tag','v0.1.0');
  f.git('checkout','main'); f.commit('feat: new feature'); f.git('checkout','-b','release/0.2');
  const [old]=await f.proposal('release/0.2',false);
  assert.equal(old.version,'0.1.0'); assert.match(old.body,/already shipped/);
  const [pr]=await f.proposal('release/0.2');
  assert.equal(pr.version,'0.2.0'); assert.match(pr.body,/new feature/); assert.doesNotMatch(pr.body,/already shipped/);
  assert.ok(pr.updates.some(update=>update.path==='.release-please-manifest.json'));
});

test('a fix-only first release follows its branch despite an old manifest',async t=>{
  const f=fixture(t,'0.2.1'); f.commit('fix: new fix'); f.git('checkout','-b','release/0.3');
  assert.equal((await f.proposal('release/0.3',false))[0].version,'0.2.2');
  assert.equal((await f.proposal('release/0.3'))[0].version,'0.3.0');
});

test('subsequent features and breaking changes stay in the line as patches',async t=>{
  const f=fixture(t,'0.2.1'); f.git('checkout','-b','release/0.3');
  f.commit('chore: release 0.3.2'); f.git('tag','-a','v0.3.2','-m','annotated stable');
  // A newer line and candidate must not replace this line's baseline.
  f.git('checkout','-b','other'); f.commit('feat: other line'); f.git('tag','v0.4.0');
  f.git('checkout','release/0.3'); f.commit('feat!: new breaking feature'); f.git('tag','v0.3.99-rc.1');
  const [pr]=await f.proposal('release/0.3');
  assert.equal(pr.version,'0.3.3'); assert.match(pr.body,/new breaking feature/); assert.doesNotMatch(pr.body,/other line/);
});

test('already shipped cherry-picks are removed by patch equivalence',async t=>{
  const f=fixture(t); f.commit('feat: already shipped'); const cut=f.git('rev-parse','HEAD');
  f.commit('chore: separate main history','unrelated.txt','main\n');
  const fix=f.commit('fix: shipped by cherry-pick','fix.txt','fixed\n');
  f.git('checkout','-b','release/0.1',cut); f.git('cherry-pick',fix);
  f.commit('chore: release 0.1.0'); f.git('tag','v0.1.0');
  f.git('checkout','main'); f.commit('fix: unshipped fix'); f.git('checkout','-b','release/0.2');
  const [pr]=await f.proposal('release/0.2');
  assert.equal(pr.version,'0.2.0'); assert.match(pr.body,/unshipped fix/); assert.doesNotMatch(pr.body,/shipped by cherry-pick|already shipped/);
});

test('an unchanged cut and a re-run at the stable tag do not propose a release',async t=>{
  const f=fixture(t); f.commit('feat: shipped'); const cut=f.git('rev-parse','HEAD');
  f.git('checkout','-b','release/0.1'); f.commit('chore: release 0.1.0'); f.git('tag','v0.1.0');
  assert.deepEqual(await f.proposal('release/0.1'),[]);
  f.git('checkout','-b','release/0.2',cut);
  assert.deepEqual(await f.proposal('release/0.2'),[]);
});

test('without a stable tag, bootstrap bounds the first release',async t=>{
  const f=fixture(t); f.git('tag','-d','v0.0.1');
  f.commit('fix: first fix'); f.git('checkout','-b','release/0.3');
  const [pr]=await f.proposal('release/0.3'); assert.equal(pr.version,'0.3.0'); assert.match(pr.body,/first fix/);
});

test('invalid branches and divergent same-line tags fail before publication',async t=>{
  const f=fixture(t); f.git('checkout','-b','release/0.3');
  const cut=f.git('rev-parse','HEAD');
  f.git('checkout','-b','diverged'); f.commit('chore: release 0.3.0'); f.git('tag','v0.3.0');
  f.git('checkout','release/0.3');
  assert.equal(f.git('rev-parse','HEAD'),cut);
  assert.throws(()=>planRelease(f.repo,'main',f.base),/release branches/);
  assert.throws(()=>planRelease(f.repo,'release/00.3',f.base),/release branches/);
  await assert.rejects(()=>run(f.github,'release/0.3',f.repo,()=>assert.fail('wrote output')),/outside this release branch/);
});

test('release outputs preserve both root and package-path publishing gates',()=>{
  assert.deepEqual(releaseOutputs([],['.']),{releases_created:false,release_created:false});
  assert.deepEqual(releaseOutputs([{path:'.',tagName:'v0.3.0'}],['.']),{releases_created:true,release_created:true,tag_name:'v0.3.0'});
  assert.deepEqual(releaseOutputs([{path:'crates/gitpm',tagName:'v0.6.0'}],['crates/gitpm']),{releases_created:true,'crates/gitpm--release_created':true,'crates/gitpm--tag_name':'v0.6.0'});
});

test('the runner opens the PR into the pushed branch with package updates',async t=>{
  const f=fixture(t,'0.2.1'); f.commit('fix: new fix'); f.git('checkout','-b','release/0.3');
  let opened;
  f.github.createPullRequest=async (pr, branch, message, updates)=>{
    opened={pr,branch,updates}; return {...pr,number:1};
  };
  const outputs={}; await run(f.github,'release/0.3',f.repo,(key,value)=>{outputs[key]=value;});
  assert.equal(opened.branch,'release/0.3'); assert.equal(opened.pr.baseBranchName,'release/0.3');
  assert.match(opened.pr.body,/0\.3\.0/); assert.match(opened.pr.body,/new fix/);
  const update=opened.updates.find(update=>update.path==='.release-please-manifest.json');
  assert.equal(JSON.parse(await update.updater.updateContent(JSON.stringify({[f.packagePath]:'0.2.1'})))[f.packagePath],'0.3.0');
  const key=f.packagePath==='.' ? 'release_created' : `${f.packagePath}--release_created`;
  assert.equal(outputs[key],false);
});

test('publishing a merged release uses its version and the original manifest',async t=>{
  const f=fixture(t,'0.2.1'); f.commit('fix: new fix'); f.git('checkout','-b','release/0.3');
  const {manifest}=await releaseManifest(f.github,'release/0.3',f.repo);
  const [pr]=await manifest.buildPullRequests();
  f.put('.release-please-manifest.json',JSON.stringify({[f.packagePath]:'0.3.0'}));
  const sha=f.commit('chore: release 0.3.0');
  f.github.pullRequestIterator=async function* (branch,status) {
    if(status==='MERGED') yield {number:7,title:pr.title.toString(),body:pr.body.toString(),headBranchName:pr.headRefName,baseBranchName:branch,sha,labels:['autorelease: pending']};
  };
  let tagged;
  f.github.createRelease=async release=>{tagged=release;return {tagName:release.tag.toString(),sha};};
  f.github.commentOnIssue=async ()=>{};
  f.github.removeIssueLabels=async ()=>{}; f.github.addIssueLabels=async ()=>{};
  f.github.createPullRequest=()=>assert.fail('a release push must not open the next patch');
  const outputs={}; await run(f.github,'release/0.3',f.repo,(key,value)=>{outputs[key]=value;});
  assert.equal(tagged.tag.toString(),'v0.3.0'); assert.equal(tagged.sha,sha);
  const prefix=f.packagePath==='.' ? '' : `${f.packagePath}--`;
  assert.equal(outputs[`${prefix}release_created`],true); assert.equal(outputs[`${prefix}tag_name`],'v0.3.0');
});

test('workflow wiring retains publication jobs, token and full tag history',()=>{
  const workflow=readFileSync(path.join(root,'.github/workflows/release.yml'),'utf8');
  const release=workflow.split('\n  release-please:\n')[1].split(/\n  [a-z][\w-]*:\n/)[0];
  assert.match(release,/fetch-depth: 0/); assert.match(release,/persist-credentials: false/);
  assert.match(release,/BRANCH: \$\{\{ github.ref_name \}\}/);
  assert.match(release,/GH_TOKEN: \$\{\{ steps.app.outputs.token \}\}/);
  assert.match(release,/run: node .github\/release-please\/run.cjs/);
  assert.match(workflow,/\n  build:\n/); assert.match(workflow,/\n  upload:\n/);
  if (root.endsWith('gitpm')) assert.match(workflow,/\n  publish:\n/);
  else {assert.match(workflow,/\n  cdn:\n/); assert.match(release,/- id: rc\n/);}
});

test('a branch that advances beyond checkout fails before publishing',async t=>{
  const f=fixture(t); f.commit('fix: new fix'); f.git('checkout','-b','release/0.3');
  const commits=f.github.mergeCommitIterator.bind(f.github);
  f.github.mergeCommitIterator=async function* (...args) {
    yield {sha:'remote-new-head',message:'fix: concurrent push',files:[f.file]};
    yield* commits(...args);
  };
  f.github.createRelease=()=>assert.fail('published with stale history');
  await assert.rejects(()=>run(f.github,'release/0.3',f.repo,()=>assert.fail('wrote output')),/changed since checkout/);
  const {manifest}=await releaseManifest(f.github,'release/0.3',f.repo);
  await assert.rejects(()=>manifest.buildPullRequests(),/changed since checkout/);
});
