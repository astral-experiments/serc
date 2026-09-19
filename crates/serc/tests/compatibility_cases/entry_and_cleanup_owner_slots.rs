use crate::common::assert_matches_cpython_marshal;

#[test]
fn nested_binary_folds_preserve_cleanup_owners() {
    for (filename, source) in [
        (
            "normal_nested_binary_3_fusions.py",
            r"def f(resource, a, b):
    scale = (1 << 64) - 1
    try:
        value = work(a, b)
        value = work(a, b)
        value = work(a, b)
    finally:
        if resource is not None:
            resource.close()
        cleanup()
    return scale
",
        ),
        (
            "normal_nested_binary_2_fusions.py",
            r"def f(resource, a, b):
    scale = (1 << 64) - 1
    try:
        value = work(a, b)
        value = work(a, b)
    finally:
        if resource is not None:
            resource.close()
        cleanup()
    return scale
",
        ),
        (
            "normal_runtime_binary_control.py",
            r"def f(resource, a, b):
    scale = (a << 64) - 1
    try:
        value = work(a, b)
        value = work(a, b)
        value = work(a, b)
    finally:
        if resource is not None:
            resource.close()
        cleanup()
    return scale
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn entry_prologues_consume_fused_cleanup_slots() {
    for (filename, source) in [
        (
            "copied_return_unrelated_1_cells_4_fusions.py",
            r"def f(resource, a, b, x, y):
    def capture():
        return x
    try:
        value = work(a, b)
        value = work(a, b)
        value = work(a, b)
        value = work(a, b)
        return value
    finally:
        if resource is not None:
            resource.close()
        cleanup()
",
        ),
        (
            "copied_return_unrelated_2_cells_3_fusions.py",
            r"def f(resource, a, b, x, y):
    def capture():
        return x, y
    try:
        value = work(a, b)
        value = work(a, b)
        value = work(a, b)
        return value
    finally:
        if resource is not None:
            resource.close()
        cleanup()
",
        ),
        (
            "copied_return_unrelated_cell_boundary_control.py",
            r"def f(resource, a, b, x, y):
    def capture():
        return x
    if resource is None:
        return None
    try:
        value = work(a, b)
        value = work(a, b)
        value = work(a, b)
        value = work(a, b)
        return value
    finally:
        if resource is not None:
            resource.close()
        cleanup()
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn local_loads_after_comprehension_restores_keep_fresh_slots() {
    for (filename, source) in [
        (
            "async_restore_local_keyword_try_ifexp.py",
            r"async def f(factory, config, items):
    task = factory(values=[item for item in items], juice=config.juice)
    try:
        namespace = config.namespace if config.enabled else None
        async with task.use(namespace):
            return await task.run()
    except Exception:
        return None
",
        ),
        (
            "restore_two_targets_local_try_ifexp.py",
            r"def f(factory, config, items):
    task = factory(values=[left + right for left, right in items], juice=config.juice)
    try:
        namespace = config.namespace if config.enabled else None
        return task.run(namespace)
    except Exception:
        return None
",
        ),
        (
            "restore_adjacent_locals_fusion_control.py",
            r"def f(factory, config, items):
    task = factory([item for item in items], config, items)
    try:
        namespace = config.namespace if config.enabled else None
        return task.run(namespace)
    except Exception:
        return None
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn unchanged_match_markers_use_fresh_entry_slots() {
    for (filename, source) in [
        (
            "entry_match_one_cell.py",
            r#"def f(subject):
    try:
        match subject:
            case "left":
                return lambda: subject
            case _:
                return None
    except Exception:
        return None
"#,
        ),
        (
            "entry_match_free_variable.py",
            r#"def outer(subject):
    def f():
        try:
            match subject:
                case "left":
                    return 1
                case _:
                    return 0
        except Exception:
            return None
    return f
"#,
        ),
        (
            "entry_match_generator.py",
            r#"def f(subject):
    try:
        match subject:
            case "left":
                yield subject
            case _:
                yield None
    except Exception:
        yield None
"#,
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_original_truth_array_owners() {
    for (filename, source) in [
        (
            "original_truth_array_manager_loop_target.py",
            r#"def f(manager):
    total = 0
    try:
        with manager as stream:
            for item in stream:
                try:
                    value = item()
                except Exception:
                    continue
                if isinstance(value, dict):
                    choices = value.get("metadata", {}).get("choices")
                    if choices:
                        if isinstance(choices, list):
                            total += len(choices)
                        elif isinstance(choices, dict):
                            total += len(choices)
                        else:
                            total += 1
                    else:
                        total += 1
    except Exception:
        pass
    return total
"#,
        ),
        (
            "original_truth_array_folded_prefix_control.py",
            r#"def f(manager, values):
    total = 0
    with manager:
        for value in values:
            try:
                if value is not None:
                    choices = value.get("choices")
                    padding = (1, 2, 3)
                    if choices:
                        total += len(choices)
                    else:
                        total += len(padding)
            except Exception:
                continue
    return total
"#,
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}
