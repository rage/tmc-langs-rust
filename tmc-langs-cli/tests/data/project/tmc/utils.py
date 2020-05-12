import importlib
import sys


def load(pkg, method, err=None):
    if not err:
        err = '{0}.{1} does not exist!'.format(pkg, method)

    def fail(*args, **kwargs):
        raise AssertionError(err)

    try:
        return getattr(importlib.import_module(pkg), method)
    except Exception:
        return fail


def get_stdout():
    return sys.stdout.getvalue().strip()


def get_stderr():
    return sys.stderr.getvalue().strip()


def any_contains(needle, haystacks):
    any(map(lambda haystack: needle in haystack, haystacks))
