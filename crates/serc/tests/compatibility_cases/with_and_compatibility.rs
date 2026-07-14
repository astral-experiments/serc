use crate::common;

#[test]
fn matches_cpython_marshal_for_terminal_sync_with_conjunction_exit() {
    let source = r#"def is_namespace_package(path, src_extensions):
    if not is_package(path):
        return False

    init_file = path / "__init__.py"
    if not init_file.exists():
        filenames = [
            filepath
            for filepath in path.iterdir()
            if filepath.suffix.lstrip(".") in src_extensions
            or filepath.name.lower() in ("setup.cfg", "pyproject.toml")
        ]
        if filenames:
            return False
    else:
        with init_file.open("rb") as open_init_file:
            file_start = open_init_file.read(4096)
            if (
                b"first" not in file_start
                and b"second" not in file_start
                and b"third" not in file_start
                and b"fourth" not in file_start
            ):
                return False
    return True
"#;
    common::assert_matches_cpython_marshal(source, "terminal_sync_with_conjunction_exit.py");
}
