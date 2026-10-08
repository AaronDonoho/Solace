"""Run acceptance test modules with every test in its own process, in parallel.

Usage: run_parallel.py module [module...]
Each test builds its own temp root and binds port 0, so tests share nothing.
SOLACE_ACCEPTANCE_JOBS sets the worker count (default: CPU count, at most 8).
"""
import os
import subprocess
import sys
import time
import unittest
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).resolve().parent


def ids(suite):
    for item in suite:
        if isinstance(item, unittest.TestSuite):
            yield from ids(item)
        else:
            yield item.id()


def run(test):
    start = time.monotonic()
    p = subprocess.run([sys.executable, '-m', 'unittest', test], cwd=HERE, text=True, capture_output=True)
    return test, p.returncode, p.stdout + p.stderr, time.monotonic() - start


def main(modules):
    sys.path.insert(0, str(HERE))
    tests = []
    for module in modules:
        tests += ids(unittest.defaultTestLoader.loadTestsFromName(module))
    jobs = int(os.environ.get('SOLACE_ACCEPTANCE_JOBS') or min(os.cpu_count() or 2, 8))
    start = time.monotonic()
    failed = []
    with ThreadPoolExecutor(max_workers=max(jobs, 1)) as pool:
        for test, code, output, took in pool.map(run, tests):
            print(('ok    ' if code == 0 else 'FAIL  ') + test + f' ({took:.1f}s)', flush=True)
            if code != 0:
                failed.append(test)
                print(output, flush=True)
    print(f'{len(tests)} tests, {len(failed)} failed, {time.monotonic() - start:.1f}s with {jobs} workers')
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
