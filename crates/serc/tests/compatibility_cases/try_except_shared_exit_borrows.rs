use crate::common;

#[test]
fn matches_cpython_borrows_after_shared_conditional_try_exits() {
    for prefix in ["", "async "] {
        for condition in ["flag", "flag and value", "flag or value"] {
            for returned in ["value", "value.attr"] {
                let source = format!(
                    "{prefix}def probe(values, flag, work, use):\n    for item in values:\n        try:\n            value = work(item)\n            if {condition}:\n                return {returned}\n        except Exception:\n            pass\n        use(item)\n"
                );
                common::assert_matches_cpython_marshal(&source, "shared_conditional_try_exit.py");
            }
        }
    }
}

#[test]
fn matches_cpython_borrows_before_extracted_try_loop_tails() {
    for (source, filename) in [
        (
            r"def probe(items, canonical_left, canonical_right):
    for left, right in items:
        try:
            if left != canonical_left:
                left.unlink(missing_ok=True)
        except Exception:
            pass
        try:
            if right != canonical_right:
                right.unlink(missing_ok=True)
        except Exception:
            pass
",
            "consecutive_try_conditional_loop.py",
        ),
        (
            r"def probe(rows, output):
    total = 0
    for left, right in rows:
        try:
            if left is not None:
                total += int(float(left))
        except ValueError:
            pass
        try:
            if right is not None:
                output.append(float(right))
        except ValueError:
            pass
    return total
",
            "consecutive_try_none_guard_loop.py",
        ),
        (
            r"def probe(items, canonical_left, canonical_right):
    for left, right in items:
        try:
            if left != canonical_left:
                left.unlink(missing_ok=True)
            canonical_left = left
        except Exception:
            pass
        try:
            if right != canonical_right:
                right.unlink(missing_ok=True)
        except Exception:
            pass
",
            "consecutive_try_populated_exit_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn borrows_through_nested_recovery_branches_before_raising_handlers() {
    for (source, filename) in [
        (
            r"def f(flag, source, consume):
    try:
        try:
            if flag:
                value = source.first()
            else:
                value = source.second()
        except ValueError:
            value = source.fallback()
    except Exception as exc:
        consume(exc)
        raise
    return consume(value)
",
            "01_sync_nested_recovery_outer_raise.py",
        ),
        (
            r"async def f(flag, source, consume):
    try:
        try:
            if flag:
                value = await source.first()
            else:
                value = await source.second()
        except ValueError as exc:
            raise RuntimeError(exc) from exc
        except Exception as exc:
            if flag:
                raise
            value = source.fallback(exc)
            consume(value)
    except Exception:
        consume(source)
        raise
    return consume(value, source)
",
            "02_async_nested_recovery_outer_raise.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_nested_recovery_borrow_entry_boundaries() {
    for (source, filename) in [
        (
            r"def f(source, consume):
    try:
        try:
            value = source.first()
        except ValueError:
            value = source.fallback()
    except Exception as exc:
        consume(exc)
        raise
    return consume(value)
",
            "03_linear_nested_recovery_control.py",
        ),
        (
            r"def f(flag, source, consume):
    try:
        try:
            if flag:
                value = source.first()
            else:
                value = source.second()
        except ValueError:
            value = source.fallback()
        consume(value)
    except Exception as exc:
        consume(exc)
        raise
    return consume(value)
",
            "05_outer_suffix_control.py",
        ),
        (
            r"def f(flag, source, consume):
    try:
        try:
            if flag:
                while True:
                    pass
                value = source.first()
            else:
                value = source.second()
        except ValueError:
            value = source.fallback()
    except Exception:
        consume(source)
        raise
    return consume(value)
",
            "07_infinite_arm_dead_store_control.py",
        ),
        (
            r"class C:
    try:
        try:
            if flag:
                value = source.first()
            else:
                value = source.second()
        except ValueError:
            value = source.fallback()
    except Exception:
        consume(source)
        raise
    consume(value)
",
            "08_class_scope_control.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
