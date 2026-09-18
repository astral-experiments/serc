use crate::common;

#[test]
fn matches_cpython_return_copies_after_generator_branches() {
    for definition in ["def", "async def"] {
        for prefix in [
            "",
            "    with manager:\n        observe()\n",
            "    for item in items:\n        observe(item)\n",
        ] {
            for expression in [
                "tuple(item for item in value)",
                "tuple(item.copy() for item in value)",
            ] {
                let source = format!(
                    "{definition} f(flag, other, ready, value, items, manager):
{prefix}    if flag:
        if other:
            result = None
        elif ready:
            result = {expression}
        else:
            result = value
    else:
        result = value
    return result
"
                );
                common::assert_matches_cpython_marshal(&source, "return_copy_jump_slots.py");
            }
        }
    }
}

#[test]
fn keeps_direct_nested_handler_returns() {
    for definition in ["def", "async def"] {
        let source = format!(
            "{definition} collect():
    try:
        value = build()
    except Exception:
        try:
            value = build()
        except Exception as error:
            return None
    return value
"
        );
        common::assert_matches_cpython_marshal(&source, "direct_nested_handler_return.py");
    }
}

#[test]
fn keeps_authored_return_locations_after_copied_keyword_calls() {
    use std::fmt::Write as _;

    for (filename, protected, large, conditional) in [
        (
            "protected_large_keyword_return_conditional.py",
            true,
            true,
            true,
        ),
        (
            "protected_large_keyword_return_linear_control.py",
            true,
            true,
            false,
        ),
        (
            "protected_short_keyword_return_conditional_control.py",
            true,
            false,
            true,
        ),
        (
            "unprotected_large_keyword_return_conditional_control.py",
            false,
            true,
            true,
        ),
    ] {
        let indent = if protected {
            "            "
        } else {
            "        "
        };
        let keywords = if large {
            let mut keywords = String::new();
            for index in 0..36 {
                writeln!(keywords, "{indent}field{index:02}=value,").unwrap();
            }
            keywords
        } else {
            format!("{indent}initial=value,\n")
        };
        let final_value = if conditional {
            "(value if flag else None)"
        } else {
            "value"
        };
        let (prefix, suffix) = if protected {
            (
                "    try:\n        return factory(\n",
                "        )\n    except ValueError:\n        raise\n",
            )
        } else {
            ("    return factory(\n", "    )\n")
        };
        let source = format!(
            "def target(factory, flag, value):\n{prefix}{keywords}{indent}final={final_value},\n{suffix}"
        );
        common::assert_matches_cpython_marshal(&source, filename);
    }
}

#[test]
fn matches_cpython_name_return_borrowing_at_original_copy_limit() {
    for (filename, source) in [
        (
            "outer_try_iterator_return.py",
            r"async def probe(items, build, report):
    for item in items:
        try:
            try:
                result = build(item)
            except Exception as error:
                report(error)
                continue
            return result
        except Exception as error:
            report(error)
    return None
",
        ),
        (
            "iterator_only_return_control.py",
            r"async def probe(items, build, report):
    for item in items:
        try:
            result = build(item)
        except Exception as error:
            report(error)
            continue
        return result
    return None
",
        ),
        (
            "outer_try_no_iterator_control.py",
            r"async def probe(item, build, report):
    try:
        try:
            result = build(item)
        except Exception as error:
            report(error)
            return None
        return result
    except Exception as error:
        report(error)
    return None
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_break_locations_through_unlocated_return_joins() {
    for (filename, source) in [
        (
            "conditional_while_break_tuple_return_nop_target.py",
            r"def f(mode, groups):
    preview = []
    used = [0 for group in groups]
    if mode:
        last = len(groups) - 1
        while len(preview) < 3 and last >= 0:
            group = groups[last]
            index = used[last]
            if index < len(group):
                preview.append(group[index])
                used[last] += 1
            else:
                break
    else:
        while len(preview) < 3:
            added = False
            for index, group in enumerate(groups):
                if group:
                    preview.append(group[0])
                    added = True
                    if len(preview) >= 3:
                        break
            if not added:
                break
    return preview, used
",
        ),
        (
            "conditional_while_break_name_return_control.py",
            r"def f(mode, groups):
    preview = []
    used = [0 for group in groups]
    if mode:
        last = len(groups) - 1
        while len(preview) < 3 and last >= 0:
            group = groups[last]
            index = used[last]
            if index < len(group):
                preview.append(group[index])
                used[last] += 1
            else:
                break
    else:
        while len(preview) < 3:
            added = False
            for index, group in enumerate(groups):
                if group:
                    preview.append(group[0])
                    added = True
                    if len(preview) >= 3:
                        break
            if not added:
                break
    return preview
",
        ),
        (
            "conditional_while_break_larger_tuple_return_control.py",
            r"def f(mode, groups):
    preview = []
    used = [0 for group in groups]
    if mode:
        last = len(groups) - 1
        while len(preview) < 3 and last >= 0:
            group = groups[last]
            index = used[last]
            if index < len(group):
                preview.append(group[index])
                used[last] += 1
            else:
                break
    else:
        while len(preview) < 3:
            added = False
            for index, group in enumerate(groups):
                if group:
                    preview.append(group[0])
                    added = True
                    if len(preview) >= 3:
                        break
            if not added:
                break
    return preview, used, mode
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_constant_nop_return_footer_copies() {
    for (filename, source) in [
        (
            "module_if_with_two_constant_footer_target.py",
            r#"if enabled:
    with manager():
        work()
"""first footer
line
"""
"""last footer
line
"""
"#,
        ),
        (
            "module_if_bound_with_two_constant_footer_target.py",
            r#"if enabled:
    values = [item for item in source]
    with manager() as resource:
        result = resource.work(values)
"""first footer
line
"""
"""last footer
line
"""
"#,
        ),
        (
            "module_with_body_constant_footer_owner_control.py",
            r#"if enabled:
    with manager():
        work()
        """body tail
        line
        """
"""first footer
line
"""
"""last footer
line
"""
"#,
        ),
        (
            "module_if_with_three_constant_footer_budget_control.py",
            r#"if enabled:
    with manager():
        work()
"""first footer
line
"""
"""second footer
line
"""
"""last footer
line
"""
"#,
        ),
        (
            "module_if_with_large_constant_footer_control.py",
            r#"if enabled:
    with manager():
        work()
"""first footer
line
"""
"""second footer
line
"""
"""third footer
line
"""
"""last footer
line
"""
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_folded_false_footer_before_shared_return_copy() {
    for (filename, source) in [
        (
            "conditional_iterator_dead_tail_return_target.py",
            r"async def emit_rows(flag, manager, fetch, emit):
    async with manager:
        rows = await fetch()
    if flag:
        for name, value in rows:
            if value:
                emit(name, value)
    else:
        for name, value in rows:
            emit(name, value)
    if False:
        import debugger
        debugger.enable()
",
        ),
        (
            "conditional_iterator_direct_footer_control.py",
            r"async def emit_rows(flag, manager, fetch, emit):
    async with manager:
        rows = await fetch()
    if flag:
        for name, value in rows:
            if value:
                emit(name, value)
    else:
        for name, value in rows:
            emit(name, value)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn counts_retained_setup_once_for_protected_local_return_copy() {
    for (filename, source) in [
        (
            "conditional_arms_protected_local_return_target.py",
            r"def convert_key(key, text_type, special_type, report):
    name: str | None
    if isinstance(key, text_type) and key.char is not None:
        name = key.char.upper()
    elif isinstance(key, special_type):
        name = key.name
    else:
        report(key)
        return None
    try:
        return name
    except KeyError:
        return None
",
        ),
        (
            "conditional_arms_direct_local_return_control.py",
            r"def convert_key(key, text_type, special_type, report):
    name: str | None
    if isinstance(key, text_type) and key.char is not None:
        name = key.char.upper()
    elif isinstance(key, special_type):
        name = key.name
    else:
        report(key)
        return None
    return name
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn shared_conditional_joins_preserve_original_return_copy_slots() {
    for (filename, source) in [
        (
            "original_tuple_return_through_shared_if_join_target.py",
            r"def target(flag, kind, choice, items, value, end):
    if flag:
        if kind == 1:
            if choice:
                value = items[0]
            else:
                items[0] = value
        elif kind == 2:
            if choice:
                value = items[1]
            else:
                items[1] = value
    else:
        value = convert(value)
        if choice:
            value = convert(value)
    return value, end
",
        ),
        (
            "original_tuple_return_direct_if_join_control.py",
            r"def target(flag, choice, items, value, end):
    if flag:
        if choice:
            value = items[0]
        else:
            items[0] = value
    else:
        value = convert(value)
    return value, end
",
        ),
        (
            "shared_conditional_fallthrough_attribute_return_target.py",
            r"def target(self, ready, selected, enabled):
    if ready:
        if selected:
            value = build()
            if enabled:
                self.value = value
        else:
            try:
                self.value = build()
            except Exception:
                recover()
    return self.value
",
        ),
        (
            "shared_conditional_iterator_fallthrough_return_target.py",
            r"def target(self, ready, selected, values):
    if ready:
        if selected:
            self.value = build()
            if self.value is None:
                for value in values:
                    consume(value)
        else:
            self.value = build()
    return self.value
",
        ),
        (
            "shared_conditional_authored_pass_return_control.py",
            r"def target(self, ready, selected, enabled):
    if ready:
        if selected:
            if enabled:
                pass
        else:
            self.value = build()
    return self.value
",
        ),
        (
            "shared_conditional_terminating_guard_return_control.py",
            r"def target(self, ready, selected, enabled):
    if ready:
        if selected:
            if enabled:
                return self.value
        else:
            self.value = build()
    return self.value
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
