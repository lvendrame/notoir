const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {spawnSync} = require('node:child_process');
const {createHash} = require('node:crypto');

const buildScript = path.join(__dirname, '../build-media-runtime.sh');
const components = ['avcodec', 'avformat', 'avutil', 'swresample', 'swscale'];
function fixture(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'notoir-windows imports-'));
  t.after(() => fs.rmSync(dir, {recursive: true, force: true}));
  fs.mkdirSync(path.join(dir, 'bin'));
  return dir;
}
function imports(dir, location = 'bin') {
  fs.mkdirSync(path.join(dir, location), {recursive: true});
  for (const name of components) fs.writeFileSync(path.join(dir, location, `${name}.lib`), `import:${name}`);
}
function prepare(dir, script = buildScript) {
  return spawnSync('bash', ['-euc', 'source "$1"\nprepare_windows_import_libraries "$2"',
    'notoir-import-test', script, dir], {encoding: 'utf8'});
}
function succeeds(result) {
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
}
function rustTools() {
  const sysroot = spawnSync('rustc', ['--print', 'sysroot'], {encoding: 'utf8'});
  const version = spawnSync('rustc', ['-vV'], {encoding: 'utf8'});
  succeeds(sysroot); succeeds(version);
  const host = version.stdout.match(/^host: (.+)$/m)[1];
  const suffix = process.platform === 'win32' ? '.exe' : '';
  const root = sysroot.stdout.trim();
  const linker = path.join(root, 'lib/rustlib', host, 'bin', `rust-lld${suffix}`);
  assert.ok(fs.existsSync(linker), 'Rust LLVM linker is required for the local COFF regressions');
  return {root, linker, compiler: path.join(root, 'bin', `rustc${suffix}`)};
}

test('Windows import libraries are staged in lib while DLLs remain in bin', t => {
  const dir = fixture(t);
  imports(dir);
  fs.writeFileSync(path.join(dir, 'bin/avcodec-63.dll'), 'runtime');
  succeeds(prepare(dir));
  for (const name of components) {
    assert.equal(fs.readFileSync(path.join(dir, 'lib', `${name}.lib`), 'utf8'), `import:${name}`);
    assert.ok(fs.existsSync(path.join(dir, 'bin', `${name}.lib`)));
  }
  assert.equal(fs.readFileSync(path.join(dir, 'bin/avcodec-63.dll'), 'utf8'), 'runtime');
  succeeds(prepare(dir));
});

test('already-normalized imports remain usable without copies in bin', t => {
  const dir = fixture(t);
  imports(dir, 'lib');
  succeeds(prepare(dir));
  for (const name of components)
    assert.equal(fs.readFileSync(path.join(dir, 'lib', `${name}.lib`), 'utf8'), `import:${name}`);
});

test('missing and empty import libraries fail before partial staging', t => {
  const dir = fixture(t);
  fs.writeFileSync(path.join(dir, 'bin/avcodec.lib'), '');
  const result = prepare(dir);
  assert.ifError(result.error);
  assert.notEqual(result.status, 0);
  for (const name of components) assert.ok(result.stderr.includes(`${name}.lib`), result.stderr);
  assert.ok(!fs.existsSync(path.join(dir, 'lib')));
});

function cachedFixture(t, omitLibrary = false) {
  const dir = fixture(t);
  fs.mkdirSync(path.join(dir, 'scripts/patches'), {recursive: true});
  const script = fs.readFileSync(buildScript);
  const patchFile = path.join(__dirname, '../patches/ffmpeg-9.1-warning-fixes.patch');
  const patch = fs.readFileSync(patchFile);
  fs.writeFileSync(path.join(dir, 'scripts/build-media-runtime.sh'), script);
  fs.writeFileSync(path.join(dir, 'scripts/patches/ffmpeg-9.1-warning-fixes.patch'), patch);
  const prefix = path.join(dir, 'target/native/ffmpeg/x86_64-pc-windows-msvc');
  fs.mkdirSync(path.join(prefix, 'bin'), {recursive: true});
  imports(prefix);
  if (omitLibrary) fs.unlinkSync(path.join(prefix, 'bin/swscale.lib'));
  const stamp = createHash('sha256').update(script).update(patch).digest('hex');
  fs.writeFileSync(path.join(prefix, 'notoir-build-id'), `${stamp}\n`);
  const archive = path.resolve(__dirname, '../../target/native/ffmpeg-9.1.tar.xz');
  assert.ok(fs.existsSync(archive), 'build the native runtime first to obtain the checksum-pinned source archive');
  return {dir, prefix, stamp, archive};
}

test('cached Windows builds repair import layout before reporting readiness', t => {
  const {dir, prefix, archive} = cachedFixture(t);
  const result = spawnSync('bash', [path.join(dir, 'scripts/build-media-runtime.sh'), 'x86_64-pc-windows-msvc'], {
    encoding: 'utf8', env: {...process.env, FFMPEG_SOURCE_ARCHIVE: archive},
  });
  succeeds(result);
  assert.match(result.stdout, /Media runtime ready:/);
  for (const name of components) assert.ok(fs.statSync(path.join(prefix, 'lib', `${name}.lib`)).size > 0);
});

test('incomplete Windows cache cannot publish readiness or a new success stamp', t => {
  const {dir, prefix, archive} = cachedFixture(t, true);
  const marker = path.join(prefix, 'notoir-build-id');
  const before = fs.statSync(marker, {bigint: true}).mtimeNs;
  const result = spawnSync('bash', [path.join(dir, 'scripts/build-media-runtime.sh'), 'x86_64-pc-windows-msvc'], {
    encoding: 'utf8', env: {...process.env, FFMPEG_SOURCE_ARCHIVE: archive},
  });
  assert.ifError(result.error);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /swscale\.lib/);
  assert.doesNotMatch(result.stdout, /Media runtime ready:/);
  assert.equal(fs.statSync(marker, {bigint: true}).mtimeNs, before);
  assert.ok(!fs.existsSync(path.join(dir, 'target/native/media-env.sh')));
});

test('real Windows COFF linking succeeds only after import libraries are staged', t => {
  const dir = fixture(t);
  const {linker} = rustTools();
  for (const name of components) {
    const source = path.join(dir, `${name}.c`);
    const object = path.join(dir, `${name}.obj`);
    fs.writeFileSync(source, `__declspec(dllexport) unsigned ${name}_version(void) { return 1U; }\n`);
    succeeds(spawnSync('clang', ['--target=x86_64-pc-windows-msvc', '-Wall', '-Wextra', '-Werror',
      '-fno-stack-protector', '-c', source, '-o', object], {encoding: 'utf8'}));
    succeeds(spawnSync(linker, ['-flavor', 'link', '/dll', '/noentry', '/nodefaultlib', '/machine:x64',
      `/out:${path.join(dir, 'bin', `${name}-fixture.dll`)}`,
      `/implib:${path.join(dir, 'bin', `${name}.lib`)}`, object], {encoding: 'utf8'}));
  }
  const main = path.join(dir, 'main.c');
  fs.writeFileSync(main, components.map(name => `__declspec(dllimport) unsigned ${name}_version(void);`).join('\n') +
    '\nvoid mainCRTStartup(void) {\n' + components.map(name => `    ${name}_version();`).join('\n') + '\n}\n');
  const object = path.join(dir, 'main.obj');
  succeeds(spawnSync('clang', ['--target=x86_64-pc-windows-msvc', '-Wall', '-Wextra', '-Werror',
    '-fno-stack-protector', '-c', main, '-o', object], {encoding: 'utf8'}));
  const output = path.join(dir, 'probe.exe');
  const args = ['-flavor', 'link', '/nodefaultlib', '/machine:x64', '/subsystem:console',
    '/entry:mainCRTStartup', `/out:${output}`, `/libpath:${path.join(dir, 'lib')}`,
    object, ...components.map(name => `${name}.lib`)];
  const missing = spawnSync(linker, args, {encoding: 'utf8'});
  assert.ifError(missing.error);
  assert.notEqual(missing.status, 0);
  assert.match(missing.stderr, /avcodec\.lib/);
  succeeds(prepare(dir));
  succeeds(spawnSync(linker, args, {encoding: 'utf8'}));
  const executable = fs.readFileSync(output);
  assert.equal(executable.toString('ascii', 0, 2), 'MZ');
  assert.equal(executable.readUInt16LE(executable.readUInt32LE(0x3c) + 4), 0x8664);
  for (const name of components) assert.ok(executable.includes(Buffer.from(`${name}-fixture.dll`)));
});

test('Rust MSVC linker identity separates progress from genuine diagnostics', t => {
  const dir = fixture(t);
  const {root, linker, compiler} = rustTools();
  assert.ok(fs.existsSync(path.join(root, 'lib/rustlib/x86_64-pc-windows-msvc/lib')),
    'run rustup target add x86_64-pc-windows-msvc before the native regression suite');
  const wrapper = path.join(dir, 'bin/link.exe');
  fs.writeFileSync(wrapper, '#!/usr/bin/env node\n' +
    "const {spawnSync} = require('node:child_process');\n" +
    'console.log(process.env.NOTOIR_TEST_LINK_OUTPUT);\n' +
    `const result = spawnSync(${JSON.stringify(linker)}, ['-flavor', 'link', ...process.argv.slice(2)], {stdio: 'inherit'});\n` +
    'if (result.error) { console.error(result.error); process.exit(1); }\n' +
    'process.exit(result.status ?? 1);\n', {mode: 0o755});
  const source = path.join(dir, 'probe.rs');
  fs.writeFileSync(source, '#![no_std]\n#![no_main]\n' +
    "use core::panic::PanicInfo;\n#[panic_handler]\nfn panic(_: &PanicInfo<'_>) -> ! { loop {} }\n" +
    '#[unsafe(no_mangle)]\npub extern "C" fn notoir_entry() -> ! { loop {} }\n');
  const progress = '   Creating library probe.lib and object probe.exp';
  const warning = 'LINK : warning LNK4099: intentional regression diagnostic';
  for (const [name, configured, message, expected] of [
    ['absolute-progress', wrapper, progress, false],
    ['bare-progress', 'link.exe', progress, true],
    ['bare-warning', 'link.exe', warning, false],
  ]) {
    const output = path.join(dir, `${name}.exe`);
    const result = spawnSync(compiler, ['--edition', '2024', '--target', 'x86_64-pc-windows-msvc',
      '-D', 'warnings', '-D', 'linker_messages', '-C', 'panic=abort', '-C', 'default-linker-libraries=no',
      '-C', `linker=${configured}`, '-C', 'link-arg=/ENTRY:notoir_entry',
      '-C', 'link-arg=/SUBSYSTEM:CONSOLE', '-C', 'link-arg=/NODEFAULTLIB', source, '-o', output], {
      encoding: 'utf8', env: {...process.env, PATH: path.join(dir, 'bin') + path.delimiter + process.env.PATH,
        NOTOIR_TEST_LINK_OUTPUT: message},
    });
    assert.ifError(result.error);
    if (expected) {
      succeeds(result);
      assert.equal(fs.readFileSync(output).toString('ascii', 0, 2), 'MZ');
      assert.doesNotMatch(result.stderr, /warning:|error:/);
    } else {
      assert.notEqual(result.status, 0, result.stderr);
      assert.match(result.stderr, /linker stdout/);
      assert.match(result.stderr, /linker[-_]messages/);
    }
  }
});

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
