use crate::common;

#[test]
fn empty_try_nop_owner_target() {
    let source = r#"def copy_one(row, base_dest, *, overwrite=False, verbose=False):
    src_uri = normalize_uri((row.get("path") or "").strip())
    if not src_uri:
        return ("", "Missing path")
    dest_uri = make_dest_path(base_dest, row)
    try:
        local_src = asset(src_uri)
        if not overwrite:
            try:
                pass
            except Exception:
                pass
        with open(local_src, "rb") as fp:
            data = fp.read()
        with asset_sink(dest_uri) as local_dest:
            Path(parent_directory(local_dest)).mkdir(parents=True, exist_ok=True)
            with open(local_dest, "wb") as out:
                out.write(data)
        if verbose:
            print(f"Copied -> {dest_uri}")
        return (dest_uri, None)
    except Exception as error:
        return (dest_uri, str(error))
"#;
    common::assert_matches_cpython_marshal(source, "empty_try_nop_target.py");
}

#[test]
fn empty_try_nop_owner_nonempty_control() {
    let source = r#"def copy_one(row, base_dest, *, overwrite=False, verbose=False):
    src_uri = normalize_uri((row.get("path") or "").strip())
    if not src_uri:
        return ("", "Missing path")
    dest_uri = make_dest_path(base_dest, row)
    try:
        local_src = asset(src_uri)
        if not overwrite:
            try:
                check_destination(dest_uri)
            except Exception:
                pass
        with open(local_src, "rb") as fp:
            data = fp.read()
        with asset_sink(dest_uri) as local_dest:
            Path(parent_directory(local_dest)).mkdir(parents=True, exist_ok=True)
            with open(local_dest, "wb") as out:
                out.write(data)
        if verbose:
            print(f"Copied -> {dest_uri}")
        return (dest_uri, None)
    except Exception as error:
        return (dest_uri, str(error))
"#;
    common::assert_matches_cpython_marshal(source, "empty_try_nop_nonempty_control.py");
}

#[test]
fn empty_try_cleanup_copies_retain_outer_handler_owners() {
    for middle in [" as middle", ""] {
        for inner in [" as inner", ""] {
            for work in ["pass", "work()"] {
                let body = format!(
                    "try:\n    pass\nexcept Error as outer:\n    try:\n        {work}\n    except Error{middle}:\n        try:\n            pass\n        except Error{inner}:\n            pass\n    report(outer)\n"
                );
                for source in [body.clone(), format!("class C:\n{}", common::indent(&body))] {
                    common::assert_matches_cpython_marshal(&source, "empty_try_cleanup_owner.py");
                }
            }
        }
    }
}
