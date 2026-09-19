use crate::common::assert_matches_cpython_marshal;

#[test]
fn normal_finally_pop_aliases_preserve_original_owners() {
    for (filename, source) in [
        (
            "boolean_join_five_calls.py",
            r"def run(manager, a, b, process):
    with manager:
        try:
            flag = a and b
            touch0()
            touch1()
            touch2()
            touch3()
            touch4()
        finally:
            if process is not None:
                try:
                    process.close()
                except LookupError:
                    pass
",
        ),
        (
            "boolean_join_three_calls.py",
            r"def run(manager, a, b, process):
    with manager:
        try:
            flag = a and b
            touch0()
            touch1()
            touch2()
        finally:
            if process is not None:
                try:
                    process.close()
                except LookupError:
                    pass
",
        ),
        (
            "branch_join_five_calls_control.py",
            r"def run(manager, a, b, process):
    with manager:
        try:
            if a:
                flag = b
            else:
                flag = a
            touch0()
            touch1()
            touch2()
            touch3()
            touch4()
        finally:
            if process is not None:
                try:
                    process.close()
                except LookupError:
                    pass
",
        ),
        (
            "folded_boolean_five_calls_control.py",
            r"def run(manager, a, b, process):
    with manager:
        try:
            flag = True and a
            touch0()
            touch1()
            touch2()
            touch3()
            touch4()
        finally:
            if process is not None:
                try:
                    process.close()
                except LookupError:
                    pass
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn normal_finally_pop_aliases_keep_slot_boundaries() {
    for (filename, source) in [
        (
            "boolean_join_two_calls_control.py",
            r"def run(manager, a, b, process):
    with manager:
        try:
            flag = a and b
            touch0()
            touch1()
        finally:
            if process is not None:
                try:
                    process.close()
                except LookupError:
                    pass
",
        ),
        (
            "direct_assignment_five_calls_control.py",
            r"def run(manager, a, b, process):
    with manager:
        try:
            flag = a
            touch0()
            touch1()
            touch2()
            touch3()
            touch4()
        finally:
            if process is not None:
                try:
                    process.close()
                except LookupError:
                    pass
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn normal_finally_truth_parent_owner_before_marker_exclusion() {
    for (filename, source) in [
        (
            "normal_finally_parent_owner_formatted_paths_high_locals.py",
            r#"def f(manager, path, marker, pairs, source, catalog, snapshot_id,
      extra0, extra1, extra2, extra3, extra4, extra5, extra6, extra7, extra8,
      extra9, extra10, extra11, extra12, extra13, extra14, extra15, extra16):
    with manager:
        complete = False
        try:
            for key, value in pairs:
                save(key, value)
            save(path / "source.json", source)
            save(path / "catalog.json", catalog)
            temporary = path / f".CURRENT.{identity.new().hex}.tmp"
            write(temporary, f"{snapshot_id}\n".encode("ascii"))
            temporary.replace(path / "CURRENT")
            sync(path)
            complete = True
        finally:
            if complete:
                marker.unlink(missing_ok=True)
                sync(path)
        unlock(manager)
"#,
        ),
        (
            "normal_finally_parent_owner_formatted_paths.py",
            r#"def f(manager, path, marker, pairs, source, catalog, snapshot_id):
    with manager:
        complete = False
        try:
            for key, value in pairs:
                save(key, value)
            save(path / "source.json", source)
            save(path / "catalog.json", catalog)
            temporary = path / f".CURRENT.{identity.new().hex}.tmp"
            write(temporary, f"{snapshot_id}\n".encode("ascii"))
            temporary.replace(path / "CURRENT")
            sync(path)
            complete = True
        finally:
            if complete:
                marker.unlink(missing_ok=True)
                sync(path)
        unlock(manager)
"#,
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_shared_normal_try_break_pop_owner_boundary() {
    let source = r"def nested_try_break_pop_owner(read, ready, cancel, close, finish):
    try:
        try:
            while True:
                if cancel is not None and cancel():
                    finish()
                    return None
                if ready():
                    value = read()
                    break
        except EOFError as error:
            value = None
    finally:
        close()
        finish()
    return value
";
    assert_matches_cpython_marshal(source, "nested_try_break_pop_shared_control.py");
}

#[test]
fn matches_cpython_retained_break_outer_finally_owner() {
    for (filename, source) in [
        (
            "retained_break_normal_try_outer_finally_owner.py",
            r"def target(queue, manager, work, recover):
    while True:
        item = queue.get()
        if item is None:
            break
        try:
            while True:
                try:
                    with manager() as value:
                        work(item, value)
                    break
                except RuntimeError as error:
                    recover(error)
        except ValueError as error:
            recover(error)
        finally:
            queue.task_done()
",
        ),
        (
            "retained_break_normal_try_populated_exit_control.py",
            r"def target(queue, manager, work, recover):
    while True:
        item = queue.get()
        if item is None:
            break
        try:
            while True:
                try:
                    with manager() as value:
                        work(item, value)
                    break
                except RuntimeError as error:
                    recover(error)
            work(item, None)
        except ValueError as error:
            recover(error)
        finally:
            queue.task_done()
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}
