use crate::common;

#[test]
fn matches_cpython_runtime_prefix_same_array_target() {
    let source = r#"if path:
    try:
        with ui.progress("loading"):
            data = read_rows(path)
        if not data:
            ui.warning("empty")
        else:
            ui.success(f"Loaded {len(data)} rows from {path}")
            frame = table.make(data)
            ui.show(frame, wide=True, hidden=True)
            size = "?"
            try:
                if path.startswith("store://") and storage is not None:
                    size = storage.stat(path).size
                else:
                    size = filesystem.size(path)
            except Exception:
                pass
            ui.caption(f"Size {size}; rows {len(data)}")
    except Exception as error:
        ui.error(f"Failed: {error}")
"#;
    common::assert_matches_cpython_marshal(source, "runtime_prefix_same_array_target.py");
}

#[test]
fn matches_cpython_runtime_prefix_prior_array_control() {
    let source = r#"if path:
    try:
        with ui.progress("loading"):
            data = read_rows(path)
        ui.success(f"Loaded {len(data)} rows from {path}")
        frame = table.make(data)
        ui.show(frame, wide=True, hidden=True)
        if not data:
            ui.warning("empty")
        else:
            size = "?"
            try:
                if path.startswith("store://") and storage is not None:
                    size = storage.stat(path).size
                else:
                    size = filesystem.size(path)
            except Exception:
                pass
            ui.caption(f"Size {size}; rows {len(data)}")
    except Exception as error:
        ui.error(f"Failed: {error}")
"#;
    common::assert_matches_cpython_marshal(source, "runtime_prefix_prior_array_control.py");
}
