def comparison_one(manager, value):
    with manager:
        try:
            prepare()
            assert value == 1, "unexpected"
        finally:
            cleanup()
    return value

def call_two(manager, value):
    with manager:
        try:
            assert check(value)
        finally:
            cleanup()
            finish()
    return value

def comparison_if(git, builtin_mock_copy):
    with working_dir(builtin_mock_copy.packages_path):
        mkdirp("mockpkg_e")
        with open("mockpkg_e/package.py", "w", encoding="utf-8") as handle:
            handle.write(pkg_template.format(name="PkgE"))
    pkg("add", "mockpkg-e")
    with working_dir(builtin_mock_copy.packages_path):
        try:
            assert "A  mockpkg_e/package.py" in git("status", "--short", output=str)
        finally:
            shutil.rmtree("mockpkg_e")
            if spack.repo.PATH.repos[0]._fast_package_checker:
                spack.repo.PATH.repos[0]._fast_package_checker.invalidate()
    with pytest.raises(spack.main.SpackCommandError):
        pkg("add", "does-not-exist")

def ordinary_with_control(manager, value):
    with manager:
        consume(value)
    return value
