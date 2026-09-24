from __future__ import annotations

import os
import subprocess
import sys

from serc import find_serc_bin


def _run() -> None:
    executable = find_serc_bin()
    arguments = [executable, *sys.argv[1:]]
    if sys.platform == "win32":
        try:
            result = subprocess.run(arguments, check=False)
        except KeyboardInterrupt:
            sys.exit(2)
        sys.exit(result.returncode)
    os.execv(executable, arguments)


if __name__ == "__main__":
    _run()
