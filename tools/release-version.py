"""Select a release version and validate the selected branch or pushed tag."""

import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path

TAG = re.compile(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-(alpha|beta)\.([1-9][0-9]*))?")


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def ancestor(commit, ref):
    return subprocess.run(["git", "merge-base", "--is-ancestor", commit, ref],
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0


def next_tag(family, base, tags):
    versions = [match.groups() for tag in tags if (match := TAG.fullmatch(tag))]
    stable = [tuple(map(int, parts[:3])) for parts in versions if parts[3] is None]
    major, minor, patch = max([base, *stable])
    result = f"v{major}.{minor}.{patch + 1}"
    if family == "normal":
        return result
    prefix = f"{result}-{family}."
    number = max((int(tag[len(prefix):]) for tag in tags
                  if tag.startswith(prefix) and TAG.fullmatch(tag)), default=0)
    return f"{prefix}{number + 1}"


def main():
    branch = os.environ["GITHUB_REF"]
    commit = git("rev-parse", "HEAD")
    stable_refs = ("refs/heads/main", "refs/heads/master")
    if os.environ["GITHUB_EVENT_NAME"] == "push":
        tag = branch.removeprefix("refs/tags/")
        match = TAG.fullmatch(tag)
        if not match:
            raise SystemExit(f"Unsupported release tag: {tag}")
        prerelease = match[4] is not None
        if not prerelease and not any(ancestor(commit, ref.replace("refs/heads/", "origin/"))
                                      for ref in stable_refs):
            raise SystemExit("Stable tags must point to a commit on main or master.")
    else:
        family = os.environ["RELEASE_FAMILY"]
        if family not in ("normal", "alpha", "beta") or not branch.startswith("refs/heads/"):
            raise SystemExit("Choose a release family and a branch.")
        prerelease = family != "normal"
        if not prerelease and branch not in stable_refs:
            raise SystemExit("Stable releases must run on main or master.")
        requested = os.environ["RELEASE_COMMIT"].strip()
        if requested:
            if not re.fullmatch(r"[0-9a-fA-F]{40}", requested) or not ancestor(requested, commit):
                raise SystemExit("The commit SHA must belong to the selected branch.")
            commit = git("rev-parse", f"{requested}^{{commit}}")
        manifest = tomllib.loads(git("show", f"{commit}:Cargo.toml"))
        base = tuple(map(int, manifest["package"]["version"].split(".")))
        tag = next_tag(family, base, git("tag", "--list").splitlines())
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
        print(f"tag={tag}\ncommit={commit}\nprerelease={str(prerelease).lower()}", file=output)


def stamp(tag):
    if not TAG.fullmatch(tag):
        raise SystemExit(f"Unsupported release tag: {tag}")
    for name in ["Cargo.toml", "Cargo.lock"]:
        path = Path(name)
        contents = path.read_text(encoding="utf-8")
        pattern = r'(?m)^(name = "perfuse"\nversion = )"[^"]+"'
        contents, count = re.subn(pattern, rf'\g<1>"{tag[1:]}"', contents)
        if count != 1:
            raise SystemExit(f"Could not stamp Perfuse version in {name}")
        path.write_text(contents, encoding="utf-8")


if __name__ == "__main__":
    if len(sys.argv) == 2:
        stamp(sys.argv[1])
    else:
        main()
