"""Use legacy Python 2 builders only with Python 2.

Under Python 3, setuptools takes package metadata from pyproject.toml,
which points at the standalone mcedit2_next package inside modern/.
"""
import sys

if sys.version_info[0] < 3:
    import setup_mcedit2
    import setup_mceditlib
else:
    from setuptools import setup
    setup()
