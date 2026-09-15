use crate::common;

#[test]
fn matches_cpython_nested_tuple_return_copy_order() {
    // The four-slot tuple tail grows when copied through the later empty joins.
    // Earlier arms must use the same copy order in both plain and suspended code.
    let source = r#"def nested_tuple_return(outer, middle, inner, extra):
    if outer:
        if middle:
            if inner:
                value = "first"
            else:
                value = "second"
        else:
            value = "third"
    else:
        value = "last"
    return value, extra


async def nested_await_tuple_return(channel, outer, extra):
    initial = await channel.read("initial")
    if outer == "first":
        value = "first"
    elif outer == "nested":
        if await channel.read("middle"):
            inner = await channel.read("inner")
            if inner == initial:
                value = "second"
            else:
                value = "third"
        else:
            value = "fourth"
    elif outer == "await":
        value = await channel.read("value")
    else:
        value = "last"
    return value, extra
"#;
    common::assert_matches_cpython_marshal(source, "nested_tuple_return.py");
}

#[test]
fn matches_cpython_guarded_normal_try_return_locations() {
    // Preserve the guard and final-else NOPs while compacting copied branch NOPs.
    // Whole-marshal equality covers instruction locations and exception ownership.
    let source = r#"def protected_guard_return(command, get_result, emit):
    try:
        result = get_result()
        if command == "head":
            emit(result)
        elif command == "check":
            emit(result)
            if str(result["status"].get("state", "")).upper() != "READY":
                return 2
        elif command == "tail":
            emit(result)
        else:
            emit(None)
    except ValueError as error:
        emit(error)
        return 3
    except RuntimeError as error:
        emit(error)
        return 1
    return 0
"#;
    common::assert_matches_cpython_marshal(source, "protected_guard_return.py");
}

#[test]
fn matches_cpython_nested_call_exit_jump_owners() {
    // Nested call arms share original joins before the normal try cleanup.
    // Surviving jumps retain their protected source owner when targets are copied.
    let source = r#""""Public source-shape proposals; not compiled or validated by this investigation."""


def nested_branch_return(outer, left, right, action):
    """Shape of corpus 178903: nested calls join before the normal try cleanup."""
    try:
        if outer:
            if left or right:
                action("first")
            else:
                action("second")
        else:
            if left or right:
                action("third")
            else:
                action("fourth")
    except Exception as error:
        action(error)
        raise
    return 0


def elif_branch_return(first, second, direct, action):
    """Shape of corpus 234559: two nested normal paths share a trailing return."""
    try:
        if first:
            result = action("prepare-first")
            if direct:
                action(result)
            else:
                action("write-first", result)
        elif second:
            result = action("prepare-second")
            if direct:
                action(result)
            else:
                action("write-second", result)
        else:
            action("finish")
    except ValueError as error:
        action(str(error))
    return 0


def plain_branch_small_return(condition, action):
    """Control: the original constant-return array is only three instructions."""
    if condition:
        action("first")
    else:
        action("second")
    return 0
"#;
    common::assert_matches_cpython_marshal(source, "nested_call_exit_fixtures.py");
}

#[test]
fn matches_cpython_protected_loop_arm_jump_owners() {
    // The as-list branch keeps a protected jump across a sibling loop arm.
    // The version without a comprehension checks the same owner independently.
    let source = r#"def protected_loop_arm_return(command, items, as_list, emit):
    try:
        if command == "head":
            emit(None)
        elif command == "items":
            if not isinstance(items, list):
                raise ValueError("expected list")
            selected = [item for item in items if isinstance(item, dict)]
            if as_list:
                emit(selected)
            else:
                for item in selected:
                    emit(str(item.get("value", "")))
        elif command == "tail":
            emit(None)
        else:
            emit(False)
    except ValueError as error:
        emit(error)
        return 3
    except RuntimeError as error:
        emit(error)
        return 1
    return 0


def protected_loop_arm_without_comprehension(command, items, as_list, emit):
    try:
        if command == "head":
            emit(None)
        elif command == "items":
            if as_list:
                emit(items)
            else:
                for item in items:
                    emit(item)
        elif command == "tail":
            emit(None)
        else:
            emit(False)
    except ValueError as error:
        emit(error)
        return 3
    except RuntimeError as error:
        emit(error)
        return 1
    return 0
"#;
    common::assert_matches_cpython_marshal(source, "protected_loop_arm_return.py");
}

#[test]
fn matches_cpython_nested_try_tuple_return_copy_order() {
    let source = r#"def probe(outer, middle, inner, raw):
    value = None
    tag = "missing"
    try:
        if outer is not None:
            if middle is not None:
                if inner is not None:
                    if isinstance(raw, str) and raw:
                        value = raw
                        tag = raw
                    else:
                        tag = "empty"
                else:
                    tag = "empty"
            else:
                tag = "no_middle"
    except Exception:
        tag = "error"
    return value, tag
"#;
    common::assert_matches_cpython_marshal(source, "nested_try_tuple_return.py");
}

#[test]
fn matches_cpython_nested_try_constant_return_copy_order() {
    let source = r#"def probe(outer, middle, inner, raw):
    value = None
    tag = "missing"
    try:
        if outer is not None:
            if middle is not None:
                if inner is not None:
                    if isinstance(raw, str) and raw:
                        value = raw
                        tag = raw
                    else:
                        tag = "empty"
                else:
                    tag = "empty"
            else:
                tag = "no_middle"
    except Exception:
        tag = "error"
    return None
"#;
    common::assert_matches_cpython_marshal(source, "nested_try_constant_return_control.py");
}

#[test]
fn matches_cpython_global_call_tuple_return_original_slots() {
    let source = r"def choose(outer, first, second, other):
    if outer:
        if first:
            if second:
                report(second)
                result = 1
            else:
                result = 2
        else:
            result = 3
    else:
        result = 4
    return result, other
";
    common::assert_matches_cpython_marshal(source, "nested_complete_if_global_call_tuple_join.py");
}

#[test]
fn matches_cpython_deferred_nested_tuple_join_copy_order() {
    let source = r"def choose(category, mode, accepted, action, failed, subtype, other):
    if category == 1:
        if mode:
            result = 1
        elif accepted:
            result = 2
        else:
            result = 3
    elif category == 2:
        if mode:
            if accepted:
                result = 2
            else:
                if action:
                    result = 3
                else:
                    if failed:
                        if subtype:
                            result = 4
                        else:
                            result = 5
                    else:
                        result = 6
        else:
            if accepted:
                result = 7
            else:
                if action:
                    result = 8
                else:
                    result = 9
    elif category == 3:
        return None, None
    else:
        raise ValueError(category)
    return result, other
";
    common::assert_matches_cpython_marshal(source, "first_elif_then_deep_complete_tuple_join.py");
}

#[test]
fn matches_cpython_repeated_cleanup_copy_nop_owner() {
    let source = r#"def probe(outer, extra, middle, inner, raw):
    value = None
    tag = "missing"
    try:
        if outer is not None:
            if extra is not None:
                if middle is not None:
                    if inner is not None:
                        if isinstance(raw, str) and raw:
                            value = raw
                            tag = raw
                        else:
                            tag = "empty"
                    else:
                        tag = "empty"
                else:
                    tag = "no_middle"
            else:
                tag = "no_extra"
    except Exception:
        tag = "error"
    return value, tag
"#;
    common::assert_matches_cpython_marshal(source, "owned_nop_extra_none_guard.py");
}

#[test]
fn matches_cpython_copied_async_with_tuple_fallback_owners() {
    // Both normal and suppressed context exits copy the tuple into cleanup holes.
    // Their copied return records must retain the coroutine fallback handler.
    let source = r"async def probe(manager):
    left = []
    right = {}
    try:
        async with manager as stream:
            async for item in stream:
                if not item:
                    continue
                left.append(item)
                right[item] = item
    except Exception:
        report()
    return left, right
";
    common::assert_matches_cpython_marshal(source, "async_with_for_tuple.py");
    let source = r#"async def probe(manager, existing):
    seen = set()
    left = []
    right = {}
    try:
        async with manager as stream:
            async for item in stream:
                name = item.name
                if not name.endswith("_entry"):
                    continue
                key = item.data.get("key")
                if not key:
                    continue
                right[key] = transform(name)
                if key in seen or key in existing:
                    continue
                value = load(key)
                if value is None:
                    continue
                left.append(value)
                seen.add(key)
    except Exception:
        report()
    return left, right
"#;
    common::assert_matches_cpython_marshal(source, "async_with_for_guards_tuple.py");
}

#[test]
fn matches_cpython_authored_pass_after_exit_threading() {
    let source = r"def probe(outer, inner, keep, other):
    if outer:
        if inner:
            pass
        else:
            if keep:
                return 1
    elif other:
        return 2
    return None
";
    common::assert_matches_cpython_marshal(source, "pass_through_nested_join.py");
    let source = r"def probe(inner, keep):
    if inner:
        pass
    elif keep:
        return 1
    return None
";
    common::assert_matches_cpython_marshal(source, "pass_direct_join_control.py");
    let source = r"def probe(outer, inner, keep, other):
    if outer:
        if inner: pass
        else:
            if keep:
                return 1
    elif other:
        return 2
    return None
";
    common::assert_matches_cpython_marshal(source, "pass_same_line_nested_join_control.py");
}

#[test]
fn matches_cpython_optimized_tuple_conditional_attribute_join() {
    common::assert_matches_cpython_marshal(
        r"def probe(obj, values):
    obj.value = (
        tuple(float(value) for value in values)
        if values
        else (0.0,)
    )
",
        "optimized_tuple_conditional_attribute.py",
    );
}

#[test]
fn matches_cpython_optimized_tuple_conditional_attribute_plain_control() {
    common::assert_matches_cpython_marshal(
        r"def probe(obj, values, collect):
    obj.value = (
        collect(values)
        if values
        else (0.0,)
    )
",
        "plain_call_conditional_attribute_control.py",
    );
}

#[test]
fn matches_cpython_optimized_tuple_conditional_keyword_join() {
    common::assert_matches_cpython_marshal(
        r"def probe(values, enabled, fail):
    raise fail(
        code=1,
        values=(
            tuple(value.code for value in values)
            if enabled
            else ()
        ),
    )
",
        "optimized_tuple_conditional_keyword.py",
    );
}

#[test]
fn matches_cpython_optimized_tuple_conditional_keyword_plain_control() {
    common::assert_matches_cpython_marshal(
        r"def probe(values, enabled, fail, collect):
    raise fail(
        code=1,
        values=(
            collect(values)
            if enabled
            else ()
        ),
    )
",
        "plain_call_conditional_keyword_control.py",
    );
}
