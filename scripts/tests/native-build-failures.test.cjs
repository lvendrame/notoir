const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {spawnSync} = require('node:child_process');

test('native failure collects independent results and prevents installation', t => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'notoir-native-failure-'));
  t.after(() => fs.rmSync(dir, {recursive: true, force: true}));
  const script = fs.readFileSync(path.join(__dirname, '../build-media-runtime.sh'), 'utf8');
  const command = script.match(/^\s*(make --keep-going .*)$/m);
  assert.ok(command, 'native build must collect independent compilation diagnostics');
  fs.writeFileSync(path.join(dir, 'Makefile'), [
    '.PHONY: all broken independent install',
    'all: broken independent',
    'broken:',
    '\t@echo "intentional compiler failure" >&2',
    '\t@exit 1',
    'independent:',
    '\t@echo compiled > completed',
    'install:',
    '\t@echo installed > installed',
    '',
  ].join('\n'));
  const result = spawnSync('bash', ['-euc', `${command[1]}\nmake install`], {
    cwd: dir, encoding: 'utf8', env: {...process.env, NOTOIR_BUILD_JOBS: '2'},
  });
  assert.ifError(result.error);
  assert.notEqual(result.status, 0, 'compilation failure must remain fatal');
  assert.match(result.stderr, /intentional compiler failure/);
  assert.ok(fs.existsSync(path.join(dir, 'completed')), 'independent compilation must finish');
  assert.ok(!fs.existsSync(path.join(dir, 'installed')), 'failed build must never install');
});
