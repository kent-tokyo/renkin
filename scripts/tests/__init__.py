"""scripts/tests/ -- unittest suite for scripts/train_reranker.py.

train_reranker.py is a standalone script (not an installed package, no
scripts/__init__.py), so it isn't importable via the normal package path
`unittest discover` sets up. With `-s scripts/tests` and no `-t`, discovery
never imports this package -- so the sys.path insertion below is NOT what
makes `import train_reranker` work; each test file does its own identical
insertion before importing. This copy exists only as a fallback for a
`-t scripts` style invocation, where this package IS imported first.

Run with: python3 -m unittest discover -s scripts/tests -p "test_*.py"
"""

import os
import sys

_SCRIPTS_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if _SCRIPTS_DIR not in sys.path:
    sys.path.insert(0, _SCRIPTS_DIR)
