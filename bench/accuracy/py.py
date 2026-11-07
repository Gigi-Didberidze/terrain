"""Import-resolution accuracy for Python against Python's own import machinery.

Walks packages with importlib.machinery.PathFinder exactly as the import system does, without
executing any module code. Usage:
    compass-indexer imports <root> --lang python > imports.jsonl
    python -I bench/accuracy/py.py <root> imports.jsonl [--sample 1000]
"""
import json
import os
import sys
from importlib.machinery import PathFinder

root = os.path.abspath(sys.argv[1])
rows = [json.loads(line) for line in open(sys.argv[2], encoding="utf-8") if line.strip()]
sample = int(sys.argv[sys.argv.index("--sample") + 1]) if "--sample" in sys.argv else 1000
cache = {}


def find(module):
    """Origin file of a module inside the workspace, or None."""
    if module in cache:
        return cache[module]
    parts = module.split(".")
    path = [root]
    spec = None
    for i in range(len(parts)):
        name = ".".join(parts[: i + 1])
        try:
            spec = PathFinder.find_spec(name, path)
        except (KeyError, ImportError, ValueError):
            # Namespace-package parents not in sys.modules: treat as unresolvable.
            spec = None
        if spec is None:
            cache[module] = None
            return None
        path = list(spec.submodule_search_locations or [])
    origin = spec.origin if spec and spec.origin and spec.origin != "namespace" else None
    if origin and not os.path.abspath(origin).startswith(root):
        origin = None
    cache[module] = origin
    return origin


def package_of(path):
    parts = path[:-3].split("/") if path.endswith(".py") else path.split("/")
    return parts[:-1]


def absolute(row):
    spec = row["spec"]
    if not spec.startswith("."):
        return spec
    level = len(spec) - len(spec.lstrip("."))
    pkg = package_of(row["from"])
    if level - 1 > len(pkg):
        return None
    base = pkg[: len(pkg) - (level - 1)]
    rest = spec[level:]
    return ".".join(base + ([rest] if rest else []))


def rel(p):
    return os.path.relpath(p, root).replace(os.sep, "/")


stride = max(1, len(rows) // (sample * 3))
checked = agree = external = ours_external = 0
misses = []
extras = []
for row in rows[::stride]:
    if checked >= sample:
        break
    mod = absolute(row)
    if not mod:
        continue
    # `from a import b`: b is a submodule if one exists, else a name inside a.
    expected = []
    unresolved_names = False
    for n in row["names"]:
        sub = find(f"{mod}.{n}")
        if sub:
            expected.append(rel(sub))
        else:
            unresolved_names = True
    if unresolved_names or not row["names"]:
        m = find(mod)
        if m:
            expected.append(rel(m))
    if not expected:
        external += 1
        if row["ours"]:
            ours_external += 1
            extras.append({"from": row["from"], "spec": row["spec"], "ours": row["ours"]})
        continue
    checked += 1
    if set(expected) <= set(row["ours"]):
        agree += 1
    else:
        misses.append({"from": row["from"], "spec": row["spec"], "names": row["names"], "expected": expected, "ours": row["ours"]})

print(json.dumps({"lang": "python", "checked": checked, "agree": agree, "rate": round(agree / max(checked, 1), 4), "oracleExternal": external, "oursResolvedExternal": ours_external}, indent=1))
for m in misses[:15]:
    print("MISS", json.dumps(m))
for m in extras[:15]:
    print("EXTRA", json.dumps(m))
