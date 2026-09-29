assert platform == "win32"
assert pointer_size == 64

try:
    mkdir("build")
    announce("created")
except FileExistsError:
    announce("existing")
    for path in paths("build"):
        if path.is_file():
            path.unlink()
        else:
            rmtree(path)

announce("downloading")
version = format_version(current_version)
filename = f"runtime-{version}.zip"
url = f"https://example.invalid/{version}/{filename}"
announce(url)
response = request(url)
response.raise_for_status()
archive(response.content).extractall("build/runtime")
announce("next")
filename = "installer.zip"
url = f"https://example.invalid/{filename}"
response = request(url)
response.raise_for_status()
digest = sha256(response.content).hexdigest()
assert digest == expected_digest
archive(response.content).extractall("build/installer")
announce("done")
