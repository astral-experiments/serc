use crate::common;

#[test]
fn preserves_multiline_wildcard_guard_lines() {
    for (filename, source) in [
        (
            "multiline_walrus_first.py",
            r"def f(subject, probe):
    match subject:
        case _ if (
            found := probe(subject)
        ) is not None:
            return found
        case 1:
            return 1
        case _:
            return 0
",
        ),
        (
            "protected_multiline_wildcard.py",
            r"def f(subject, probe):
    try:
        match subject:
            case _ if (
                found := probe(subject)
            ) is not None:
                return found
            case 1:
                return 1
            case _:
                return 0
    except Exception:
        return -1
",
        ),
        (
            "same_line_walrus_control.py",
            r"def f(subject, probe):
    match subject:
        case _ if (found := probe(subject)) is not None:
            return found
        case 1:
            return 1
        case _:
            return 0
",
        ),
        (
            "final_regular_guarded_wildcard_control.py",
            r"def f(subject, probe):
    match subject:
        case _ if (
            found := probe(subject)
        ) is not None:
            return found
        case _:
            return 0
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_authored_pass_lines_at_match_exits() {
    for (filename, source) in [
        (
            "match_work_pass_name_return.py",
            r#"def f(value, result, work):
    match value:
        case "a":
            work()
            pass
        case "b":
            work()
        case _:
            raise ValueError(value)
    return result
"#,
        ),
        (
            "match_work_pass_same_line.py",
            r#"def f(value, result, work):
    match value:
        case "a":
            work(); pass
        case "b":
            work()
    return result
"#,
        ),
        (
            "match_constant_pass_same_line.py",
            r#"def f(value, result):
    match value:
        case "a":
            1; pass
        case "b":
            result.attr = 2
    return result
"#,
        ),
        (
            "match_conditional_before_pass.py",
            r#"def f(value, ready, result, work):
    match value:
        case "a":
            if ready:
                work()
            pass
        case "b":
            work()
    return result
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_original_nested_match_joins() {
    for (filename, source) in [
        (
            "match_class_nested_return.py",
            r"def f(value):
    match value:
        case Outer(item=item):
            match item:
                case Inner(data=data):
                    return data
    return None
",
        ),
        (
            "match_nested_default_pass_control.py",
            r"def f(value, result):
    match value:
        case int():
            match result:
                case str():
                    return result
                case _:
                    pass
        case _:
            pass
    return result
",
        ),
        (
            "match_class_nonempty_continuation_control.py",
            r"def f(value, work):
    match value:
        case Outer(item=item):
            match item:
                case Inner(data=data):
                    return data
            work(item)
    return None
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_match_failure_pop_block_lines() {
    for (filename, source) in [
        (
            "value_returns.py",
            r#"def f(subject, probe):
    try:
        match subject:
            case "a":
                return probe(subject)
            case "b":
                return probe(subject)
    except TypeError:
        return False
    return False
"#,
        ),
        (
            "singleton_returns.py",
            r#"def f(subject, probe):
    try:
        match subject:
            case "a":
                return probe(subject)
            case None:
                return probe(subject)
    except TypeError:
        return False
    return False
"#,
        ),
        (
            "outer_with_owner.py",
            r#"def f(subject, probe, manager):
    with manager:
        try:
            match subject:
                case "a":
                    return probe(subject)
                case "b":
                    return probe(subject)
        except TypeError:
            return False
        return False
"#,
        ),
        (
            "finally_control.py",
            r#"def f(subject, probe):
    try:
        match subject:
            case "a":
                return probe(subject)
            case "b":
                return probe(subject)
    except TypeError:
        return False
    finally:
        probe(None)
    return False
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_value_match_failure_joins_before_literal_returns() {
    for (filename, source) in [
        (
            "match_value_if_small_return.py",
            r#"def f(flag, value, work):
    if flag:
        match value:
            case "a":
                return work(value)
            case "b":
                return work(value)
    elif value:
        return work(flag)
    return None
"#,
        ),
        (
            "match_if_return_string_else.py",
            r#"def f(flag, value, work):
    if flag:
        match value:
            case "a":
                return work(value)
            case "b":
                return work(value)
    else:
        return work(flag)
    return None
"#,
        ),
        (
            "match_if_return_attribute_elif.py",
            r"def f(flag, value, work):
    if flag:
        match value:
            case Kinds.first:
                return work(value)
            case Kinds.second:
                return work(value)
    elif value:
        return work(flag)
    return None
",
        ),
        (
            "match_if_return_attribute_else.py",
            r"def f(flag, value, work):
    if flag:
        match value:
            case Kinds.first:
                return work(value)
            case Kinds.second:
                return work(value)
    else:
        return work(flag)
    return None
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_value_match_join_guard_and_continuation_boundaries() {
    for (filename, source) in [
        (
            "match_if_return_guarded_control.py",
            r"def f(flag, value, work):
    if flag:
        match value:
            case Kinds.first:
                return work(value)
            case Kinds.second if work(flag):
                return work(value)
    elif value:
        return work(flag)
    return None
",
        ),
        (
            "match_if_return_nonliteral_control.py",
            r"def f(flag, value, work):
    if flag:
        match value:
            case Kinds.first:
                return work(value)
            case Kinds.second:
                return work(value)
    elif value:
        return work(flag)
    return work(None)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_manager_pops_at_original_match_joins() {
    for (filename, source) in [
        (
            "class_as_with_returns.py",
            r"def f(subject, manager, First, Second, make):
    with manager:
        match subject:
            case First() as first:
                return make(first)
            case Second() as second:
                return make(second)
",
        ),
        (
            "sequence_with_raise_default.py",
            r"def f(subject, manager, result, make):
    with manager:
        match subject:
            case [record]:
                result.value = make(record)
                result.cursor = record.cursor
            case _:
                raise ValueError(subject)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn keeps_match_manager_donor_boundaries() {
    for (filename, source) in [
        (
            "class_as_with_guard_control.py",
            r"def f(subject, manager, First, Second, make):
    with manager:
        match subject:
            case First() as first:
                return make(first)
            case Second() as second if second.ready:
                return make(second)
",
        ),
        (
            "sequence_with_two_successes_control.py",
            r"def f(subject, manager, result, make):
    with manager:
        match subject:
            case [record]:
                result.value = make(record)
                result.cursor = record.cursor
            case [left, right]:
                result.value = left
                result.cursor = right
            case _:
                raise ValueError(subject)
",
        ),
        (
            "sequence_with_nonterminal_fallback_control.py",
            r"def f(subject, manager, result, make):
    with manager:
        match subject:
            case [record]:
                result.value = make(record)
                result.cursor = record.cursor
            case _:
                make(subject)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
