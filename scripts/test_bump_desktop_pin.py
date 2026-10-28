"""Exercise the release shell step with real Git repositories and a stub GitHub CLI."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BRANCH = 'bot/bump-tilrd'


def git(repo, *args):
    return subprocess.check_output(['git', '-C', str(repo), *args], text=True,
                                   stderr=subprocess.STDOUT).strip()


class DesktopPin(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.main = self.root / 'main'
        self.main.mkdir()
        git(self.main, 'init', '-b', 'main')
        git(self.main, 'config', 'user.name', 'Fixture')
        git(self.main, 'config', 'user.email', 'fixture@example.test')
        git(self.main, 'config', 'commit.gpgsign', 'false')
        self.pin = self.main / 'server-release.json'
        self.pin.write_text(json.dumps(dict(version='0.2.1', build=4, protocolVersion=1)) + '\n')
        (self.main / 'unrelated').write_text('keep from main\n')
        git(self.main, 'add', '.')
        git(self.main, 'commit', '-m', 'base')
        self.remote = self.root / 'remote.git'
        subprocess.check_output(['git', 'clone', '--bare', str(self.main), str(self.remote)],
                                stderr=subprocess.STDOUT)
        git(self.main, 'remote', 'add', 'origin', str(self.remote))
        self.source = self.root / 'released'
        (self.source / 'scripts').mkdir(parents=True)
        self.metadata = self.source / 'crates/tilr-proto/src/lib.rs'
        self.metadata.parent.mkdir(parents=True)
        self.metadata.write_text('pub const PROTOCOL_VERSION: u32 = 7;\n')
        self.script = self.source / 'scripts/bump-desktop-pin.sh'
        shutil.copyfile(ROOT / 'scripts/bump-desktop-pin.sh', self.script)
        self.state = self.root / 'github.json'
        self.state.write_text(json.dumps(dict(calls=[], prs=[])))
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        gh = self.bin / 'gh'
        gh.write_text('''#!/usr/bin/env python3
import json, os, subprocess, sys
from pathlib import Path
path = Path(os.environ['GH_STATE'])
state = json.loads(path.read_text())
a = sys.argv[1:]
state['calls'].append(a)
if a[:2] == ['auth', 'setup-git']:
    pass
elif a[:2] == ['repo', 'clone']:
    assert a[2] == 'tilr-dev/desktop-mac'
    subprocess.run(['git', 'clone', os.environ['DESKTOP_REMOTE'], a[3], *a[5:]], check=True)
elif a[:2] == ['pr', 'list']:
    assert a[2:] == ['--repo', 'tilr-dev/desktop-mac', '--base', 'main', '--head', 'bot/bump-tilrd', '--state', 'open', '--json', 'number', '--jq', '.[0].number // empty']
    if state['prs']:
        print(state['prs'][0]['number'])
elif a[:2] == ['pr', 'create']:
    assert not state['prs'], 'duplicate pull request'
    state['prs'].append(dict(number=42))
    state['prs'][0].update(title=a[a.index('--title')+1], body=a[a.index('--body')+1])
elif a[:2] == ['pr', 'edit']:
    assert a[2] == '42'
    state['prs'][0].update(title=a[a.index('--title')+1], body=a[a.index('--body')+1])
else:
    raise AssertionError(a)
path.write_text(json.dumps(state))
''')
        gh.chmod(0o755)
        self.env = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ['PATH'],
                        GH_STATE=str(self.state), DESKTOP_REMOTE=str(self.remote),
                        IS_RC='false', TAG='v0.3.0',
                        GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull)

    def run_step(self, success=True, **env):
        result = subprocess.run(['bash', str(self.script)], env={**self.env, **env},
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0 if success else 1, result.stdout + result.stderr)
        return result

    def read_pin(self):
        return json.loads(git(self.remote, 'show', f'{BRANCH}:server-release.json'))

    def github(self):
        return json.loads(self.state.read_text())

    def test_stable_opens_exact_pin_only_pr(self):
        self.run_step()
        self.assertEqual(self.read_pin(), dict(version='0.3.0', build=4, protocolVersion=7))
        self.assertEqual(git(self.remote, 'diff', '--name-only', 'main', BRANCH), 'server-release.json')
        pr = self.github()['prs'][0]
        self.assertEqual(pr['title'], 'chore: bump tilrd to 0.3.0')
        self.assertIn('v0.3.0', pr['body'])
        self.assertEqual(git(self.remote, 'show', f'{BRANCH}:unrelated'), 'keep from main')

    def test_next_release_updates_same_pr_from_current_main(self):
        self.run_step()
        first = git(self.remote, 'rev-parse', BRANCH)
        (self.main / 'unrelated').write_text('new main\n')
        pin = json.loads(self.pin.read_text())
        pin['build'] = 9
        self.pin.write_text(json.dumps(pin) + '\n')
        git(self.main, 'add', '.')
        git(self.main, 'commit', '-m', 'main advances')
        git(self.main, 'push', 'origin', 'main')
        self.run_step(TAG='v0.3.1')
        self.assertEqual(self.read_pin(), dict(version='0.3.1', build=9, protocolVersion=7))
        self.assertEqual(git(self.remote, 'show', f'{BRANCH}:unrelated'), 'new main')
        self.assertEqual(git(self.remote, 'rev-parse', f'{BRANCH}^'), git(self.remote, 'rev-parse', 'main'))
        self.assertNotEqual(first, git(self.remote, 'rev-parse', BRANCH))
        state = self.github()
        self.assertEqual(len(state['prs']), 1)
        self.assertEqual(state['prs'][0]['title'], 'chore: bump tilrd to 0.3.1')
        self.assertEqual(sum(c[:2] == ['pr', 'create'] for c in state['calls']), 1)
        self.assertEqual(sum(c[:2] == ['pr', 'edit'] for c in state['calls']), 1)

    def test_rerun_reuses_existing_pr(self):
        self.run_step()
        self.run_step()
        self.assertEqual(len(self.github()['prs']), 1)
        self.assertEqual(self.read_pin()['version'], '0.3.0')

    def test_older_or_equal_release_preserves_open_newer_pr(self):
        self.run_step(TAG='v0.10.0')
        before = git(self.remote, 'rev-parse', BRANCH)
        pr = self.github()['prs']
        for tag in ['v0.3.2', 'v0.10.0']:
            with self.subTest(tag=tag):
                result = self.run_step(TAG=tag)
                self.assertIn('not newer', result.stdout)
                self.assertEqual(git(self.remote, 'rev-parse', BRANCH), before)
                self.assertEqual(self.github()['prs'], pr)
        self.run_step(TAG='v0.10.1')
        self.assertEqual(self.read_pin()['version'], '0.10.1')

    def test_older_or_equal_release_preserves_newer_main(self):
        pin = dict(version='1.0.0', build=4, protocolVersion=9)
        self.pin.write_text(json.dumps(pin) + '\n')
        git(self.main, 'add', '.')
        git(self.main, 'commit', '-m', 'newer main pin')
        git(self.main, 'push', 'origin', 'main')
        before = git(self.remote, 'rev-parse', 'main')
        for tag in ['v0.3.2', 'v1.0.0']:
            with self.subTest(tag=tag):
                result = self.run_step(TAG=tag)
                self.assertIn('not newer', result.stdout)
                self.assertEqual(git(self.remote, 'branch', '--list', BRANCH), '')
                self.assertEqual(git(self.remote, 'rev-parse', 'main'), before)
                self.assertFalse(self.github()['prs'])

    def test_newer_main_leaves_older_open_proposal_unchanged(self):
        self.run_step(TAG='v0.3.0')
        before = git(self.remote, 'rev-parse', BRANCH)
        pr = self.github()['prs']
        self.pin.write_text(json.dumps(dict(version='0.4.0', build=4, protocolVersion=9)))
        git(self.main, 'add', '.')
        git(self.main, 'commit', '-m', 'main already advanced')
        git(self.main, 'push', 'origin', 'main')
        self.run_step(TAG='v0.4.0')
        self.assertEqual(git(self.remote, 'rev-parse', BRANCH), before)
        self.assertEqual(self.github()['prs'], pr)

    def test_closed_pr_branch_does_not_block_new_proposal(self):
        self.run_step(TAG='v1.0.0')
        state = self.github()
        state['prs'] = []
        self.state.write_text(json.dumps(state))
        self.run_step(TAG='v0.3.2')
        self.assertEqual(self.read_pin()['version'], '0.3.2')
        self.assertEqual(len(self.github()['prs']), 1)

    def test_invalid_artifact_revision_does_not_publish(self):
        self.pin.write_text(json.dumps(dict(version='0.2.1', build=0, protocolVersion=1)))
        git(self.main, 'add', '.')
        git(self.main, 'commit', '-m', 'invalid pin')
        git(self.main, 'push', 'origin', 'main')
        self.run_step(success=False)
        self.assertEqual(git(self.remote, 'branch', '--list', BRANCH), '')
        self.assertFalse(self.github()['prs'])

    def test_candidates_and_no_release_do_nothing(self):
        self.run_step(TAG='v0.3.0-rc.1', IS_RC='true')
        self.run_step(TAG='')
        self.assertEqual(self.github()['calls'], [])
        self.assertEqual(git(self.remote, 'branch', '--list', BRANCH), '')

    def test_invalid_stable_tag_fails_before_github(self):
        self.run_step(success=False, TAG='v0.3.0-rc.1')
        self.assertEqual(self.github()['calls'], [])

    def test_missing_invalid_or_ambiguous_protocol_fails_before_github(self):
        for metadata in ['', 'pub const PROTOCOL_VERSION: u32 = 0;\n',
                         'pub const PROTOCOL_VERSION: u32 = 4294967296;\n',
                         'pub const PROTOCOL_VERSION: u32 = 1;\n' * 2]:
            with self.subTest(metadata=metadata):
                self.metadata.write_text(metadata)
                self.run_step(success=False)
                self.assertEqual(self.github()['calls'], [])

    def test_same_pin_on_main_needs_no_pr(self):
        self.metadata.write_text('pub const PROTOCOL_VERSION: u32 = 1;\n')
        self.state.write_text(json.dumps(dict(calls=[], prs=[])))
        self.run_step(TAG='v0.2.1')
        self.assertFalse(self.github()['prs'])
        self.assertFalse(any(c[:2] in [['pr', 'create'], ['pr', 'edit']]
                             for c in self.github()['calls']))


if __name__ == '__main__':
    unittest.main()
