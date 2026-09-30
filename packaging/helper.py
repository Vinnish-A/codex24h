"""One bundled runtime shared by the four installed helper entry points."""
import os
from pathlib import Path
import runpy
import sys

# PyInstaller's private libraries must not override libraries of tmux / Codex.
original = os.environ.pop('LD_LIBRARY_PATH_ORIG', None)
if original is None:
    os.environ.pop('LD_LIBRARY_PATH', None)
else:
    os.environ['LD_LIBRARY_PATH'] = original

names = ('mail', 'attach', 'requests', 'session')
name = Path(sys.argv[0]).name.removeprefix('codex24h-')
if os.environ.get('CODEX24H_TMUX_CLIENT'):
    name = 'attach'
if sys.argv[1:] and sys.argv[1] in names:
    name = sys.argv.pop(1)
if name not in names:
    raise SystemExit('Use codex24h-mail, codex24h-attach, codex24h-requests or codex24h-session')
runpy.run_path(str(Path(__file__).parent / 'scripts' / ('codex24h-' + name)), run_name='__main__')
