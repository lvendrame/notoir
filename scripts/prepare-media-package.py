#!/usr/bin/env python3
"""Generate absolute cargo-packager config with self-contained media resources."""
import copy
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys

root = Path(__file__).resolve().parent.parent
os.chdir(root)
os_name, binaries, runtime = sys.argv[1:]
runtime = Path(runtime).resolve()
stage = root / 'target/native/media-payload'
stage.mkdir(parents=True, exist_ok=True)
# Clear only this dedicated generated staging folder.
for item in stage.iterdir():
    if item.is_file() or item.is_symlink():
        item.unlink()
patterns = ['*.dll'] if os_name == 'windows' else ['*.dylib'] if os_name == 'macos' else ['*.so*']
libdir = runtime / ('bin' if os_name == 'windows' else 'lib')
names = ('avcodec','avformat','avutil','swresample','swscale')
libs = sorted({p for pattern in patterns for p in libdir.glob(pattern)
               if any(p.name.startswith(name if os_name == 'windows' else 'lib'+name) for name in names)
               and (os_name == 'windows' or re.search(r'^lib[a-z]+\.\d+\.dylib$' if os_name == 'macos' else r'\.so\.\d+$',p.name))})
for name in names:
    if not any(p.name.startswith(name if os_name == 'windows' else 'lib'+name) for p in libs):
        raise SystemExit(f'Missing required FFmpeg library: {name}')
if not libs:
    raise SystemExit(f'No bundled media libraries in {libdir}')
for path in libs:
    shutil.copyfile(path, stage / path.name)
for name in ['FFmpeg-LICENSE.txt', 'FFmpeg-BUILD.txt']:
    shutil.copyfile(runtime / name, stage / name)
manifest_dir = root / 'apps/notoir'
# `cargo metadata` instead of tomllib: Ubuntu 22.04 release runners ship Python 3.10.
metadata = json.loads(subprocess.run(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                     check=True, capture_output=True, text=True).stdout)
package = next(p for p in metadata['packages'] if p['name'] == 'notoir')
config = copy.deepcopy(package['metadata']['packager'])
config['version'] = package['version']
config['name'] = 'notoir'
config['out-dir'] = str(root/'target/packages')
config['binaries-dir'] = str(Path(binaries).resolve())
config['icons'] = [str((manifest_dir/p).resolve()) for p in config['icons']]
config['nsis']['template'] = str((manifest_dir/config['nsis']['template']).resolve())
config['resources'] = [{'src':str(p), 'target':p.name if os_name == 'windows' else 'media/'+p.name} for p in sorted(stage.iterdir())]
if os_name == 'linux':
    # Linker rpath matches /usr/lib/notoir/media in both .deb and AppDir.
    subprocess.run(['patchelf','--set-rpath','$ORIGIN/../lib/notoir/media',str(Path(binaries)/'notoir')],check=True)
    for path in stage.glob('*.so*'):
        subprocess.run(['patchelf','--set-rpath','$ORIGIN',str(path)],check=True)
output = root/'target/native/media-packager.json'
output.write_text(json.dumps(config,indent=2)+'\n')
print(output)
