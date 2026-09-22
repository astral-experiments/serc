use crate::common;

#[test]
fn matches_cpython_nested_handler_body_cleanup_copies() {
    for (header, prefix) in [
        ("def", ""),
        ("async def", "    await suspend()\n"),
        ("def", "    yield value\n"),
    ] {
        for body in ["result = work(value)", "work(value)"] {
            for following in ["return result", "finish(result)"] {
                for normal_else in ["", "        else:\n            result = other(value)\n"] {
                    let source = format!(
                        "{header} f(value):\n{prefix}    try:\n        result = work(value)\n    except ValueError:\n        try:\n            {body}\n        except TypeError:\n            raise\n{normal_else}    {following}\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "nested_handler_cleanup_copies.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_nested_handler_else_join_controls() {
    for (header, prefix) in [
        ("def", ""),
        ("async def", "    await suspend()\n"),
        ("def", "    yield value\n"),
    ] {
        for body in [
            "            with manager():\n                result = work(value)\n",
            "            match value:\n                case 1:\n                    result = work(value)\n                case _:\n                    result = other(value)\n",
        ] {
            let source = format!(
                "{header} f(value):\n{prefix}    try:\n        result = work(value)\n    except ValueError:\n        try:\n{body}        except TypeError:\n            raise\n        else:\n            result = other(value)\n    return result\n"
            );
            common::assert_matches_cpython_marshal(&source, "nested_handler_else_joins.py");
        }
    }
}

#[test]
fn matches_cpython_nested_terminating_else_cleanup_locations() {
    for header in ["def", "async def"] {
        for prefix in ["", "                    prepare()\n"] {
            let source = format!(
                "{header} f(flag, outer, inner):\n    try:\n        work()\n    except Error as error:\n        if outer:\n            if inner:\n                action()\n            else:\n                if flag:\n{prefix}                    raise Other() from error\n        else:\n            raise\n    following()\n    return flag\n"
            );
            common::assert_matches_cpython_marshal(&source, "nested_terminating_else_cleanup.py");
        }
    }
}

#[test]
fn matches_cpython_nested_terminating_else_surviving_jumps() {
    for condition in ["flag", "not flag", "flag or other", "flag < other"] {
        for terminal in ["raise Other()", "raise", "return None"] {
            let source = format!(
                "def f(flag, outer, inner, other, manager, values):\n    if outer:\n        if inner:\n            action()\n        else:\n            if {condition}:\n                {terminal}\n    else:\n        raise Other()\n    following()\n"
            );
            common::assert_matches_cpython_marshal(&source, "nested_terminating_else_jump.py");
        }
    }
}

#[test]
fn keeps_nested_nonterminating_else_locations() {
    for body in [
        "                with manager:\n                    action()\n",
        "                try:\n                    action()\n                except Other:\n                    recover()\n",
    ] {
        let source = format!(
            "def f(flag, outer, inner, other, manager, values):\n    if outer:\n        if inner:\n            action()\n        else:\n            if flag:\n{body}    else:\n        raise Other()\n    following()\n"
        );
        common::assert_matches_cpython_marshal(&source, "nested_nonterminating_else_control.py");
    }
}

#[test]
fn preserves_nested_named_cleanup_copy_owners_at_loop_tail() {
    // The second POP_EXCEPT leaves the outer handler. Its copied tail belongs
    // to the scope fallback in an async function and has no owner in a normal one.
    for (header, suspend, inner_binding, error) in [
        ("def", "", "", "primary_error"),
        ("def", "", " as repair_error", "repair_error"),
        ("async def", "await ", " as repair_error", "repair_error"),
    ] {
        let source = format!(
            "{header} f(values, retry, skip):\n    errors = {{}}\n    pending = []\n    for value in values:\n        try:\n            primary(value)\n        except ValueError as primary_error:\n            if skip:\n                errors[value] = primary_error\n                if retry:\n                    pending.append(value)\n                continue\n            try:\n                {suspend}repair(value)\n            except TypeError{inner_binding}:\n                errors[value] = {error}\n                if retry:\n                    pending.append(value)\n    return pending\n"
        );
        common::assert_matches_cpython_marshal(
            &source,
            "nested_named_cleanup_copy_owners_at_loop_tail.py",
        );
    }
}

#[test]
fn matches_cpython_fresh_condition_slot_in_nested_retained_try() {
    for header in ["def", "async def"] {
        let source = format!(
            "{header} f(items, service, enabled):\n    for item in items:\n        try:\n            if enabled:\n                try:\n                    removed = service.remove(item)\n                    if not removed:\n                        service.failed(item)\n                    else:\n                        replacement = service.create(item)\n                        if replacement is None:\n                            service.failed(item)\n                        else:\n                            service.ready(replacement)\n                except Exception as error:\n                    service.failed(error)\n        except Exception as error:\n            service.failed(error)\n"
        );
        common::assert_matches_cpython_marshal(&source, "nested_retained_try_condition.py");
    }
}

#[test]
fn preserves_named_handler_return_owner_before_older_iterators() {
    for (filename, source) in [
        (
            "named_handler_return_before_outer_iterator.py",
            "def probe(values, run, retry, recover, close):\n    for value in values:\n        try:\n            try:\n                run(value)\n            except KeyError as error:\n                if retry(error):\n                    continue\n                return recover(error)\n        except RuntimeError:\n            run(value)\n        finally:\n            close(value)\n",
        ),
        (
            "named_handler_tuple_return_before_outer_iterator.py",
            "def probe(values, run, retry, recover, close):\n    for value in values:\n        try:\n            try:\n                run(value)\n            except KeyError as error:\n                if retry(error):\n                    continue\n                return recover(error), value\n        except RuntimeError:\n            run(value)\n        finally:\n            close(value)\n",
        ),
        (
            "named_handler_no_iterator_control.py",
            "def probe(value, run, recover, close):\n    try:\n        try:\n            run(value)\n        except KeyError as error:\n            return recover(error)\n    except RuntimeError:\n        run(value)\n    finally:\n        close(value)\n",
        ),
        (
            "named_handler_younger_iterator_control.py",
            "def probe(values, run, recover, close):\n    try:\n        try:\n            for value in values:\n                try:\n                    run(value)\n                except KeyError as error:\n                    return recover(error)\n        except RuntimeError:\n            run(None)\n    finally:\n        close(None)\n",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_shared_unnamed_handler_local_return_copies() {
    for (filename, source) in [
        (
            "handler_manager_simple_recovery_control.py",
            r"def read_cached(kind, open_resource, load, build, save):
    try:
        if kind == 'text':
            with open_resource('r') as handle:
                result = load(handle)
        elif kind == 'bytes':
            with open_resource('rb') as handle:
                result = load(handle)
        else:
            raise ValueError('unknown format')
    except FileNotFoundError:
        result = build()
        save(result)
    return result
",
        ),
        (
            "nested_handler_loop_shared_local_return_target.py",
            r"def collect_entries(data):
    entries = []
    try:
        for key in data['sources'].keys():
            item = data['sources'][key]
            entries.append({'value': item['value'], 'kind': item['kind']})
    except Exception:
        try:
            for item in data['sources']:
                entries.append({'value': item['value'], 'kind': item['kind']})
        except Exception:
            try:
                for key in data.keys():
                    item = data[key]
                    entries.append({'value': item['value'], 'kind': item['kind']})
            except Exception:
                pass
    return entries
",
        ),
        (
            "nested_handler_simple_recovery_control.py",
            r"def collect_entries(data):
    entries = []
    try:
        for key in data['sources'].keys():
            item = data['sources'][key]
            entries.append({'value': item['value'], 'kind': item['kind']})
    except Exception:
        pass
    return entries
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_nested_handler_shared_global_return_copies() {
    for returned in ["value", "other"] {
        for prefix in ["", "                    notify()\n"] {
            let source = format!(
                "def f():\n    global value\n    if value is None:\n        try:\n            value = first()\n        except Error:\n            try:\n                with manager():\n{prefix}                    value = second()\n            except Error:\n                value = third()\n                done(value)\n    return {returned}\n"
            );
            common::assert_matches_cpython_marshal(&source, "nested_handler_global_return.py");
        }
    }
}

#[test]
fn matches_cpython_nested_manager_terminating_handler_return_copies() {
    for (filename, source) in [
        (
            "nested_fallback_manager_tuple_return_target.py",
            r"def read_size(open_resource, open_image, read_bytes, report):
    try:
        with open_resource() as resource:
            with open_image(resource) as image:
                image.load()
                size = (int(image.width), int(image.height))
    except Exception:
        try:
            data = read_bytes()
            with open_image(data) as image:
                image.load()
                size = (int(image.width), int(image.height))
        except Exception as error:
            report(error)
            return None
    return size
",
        ),
        (
            "nested_fallback_direct_tuple_return_control.py",
            r"def read_size(open_resource, open_image, read_bytes, report):
    try:
        with open_resource() as resource:
            with open_image(resource) as image:
                image.load()
                size = (int(image.width), int(image.height))
    except Exception:
        try:
            data = read_bytes()
            image = open_image(data)
            image.load()
            size = (int(image.width), int(image.height))
        except Exception as error:
            report(error)
            return None
    return size
",
        ),
        (
            "nested_fallback_direct_shared_return_control.py",
            r"def read_fallback(open_resource, read, report):
    try:
        with open_resource('primary') as handle:
            result = read(handle)
    except ValueError:
        try:
            result = read(open_resource('fallback'))
        except ValueError:
            report()
            result = []
    return result
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_threaded_named_handler_small_return_copies() {
    for (filename, source) in [
        (
            "named_handler_direct_cleanup_small_return_control.py",
            r"def recover(fetch, report, choose):
    result = None
    try:
        result = fetch()
    except Exception as error:
        if choose:
            report(error)
        else:
            report(error)
    return result
",
        ),
        (
            "named_handler_inherited_nested_if_call_return_control.py",
            r"def update(callback, report, types, state, displayed):
    if displayed:
        try:
            callback()
        except Exception as error:
            if types:
                matches = [index for index, kind in enumerate(types) if isinstance(error, kind)]
                if matches:
                    state[0] = matches[0] + 1
                else:
                    report(error)
            else:
                report(error)
    return finish(displayed)
",
        ),
        (
            "named_handler_inherited_nested_if_small_return_target.py",
            r"def update(callback, report, types, state, displayed):
    if displayed:
        try:
            callback()
        except Exception as error:
            if types:
                matches = [index for index, kind in enumerate(types) if isinstance(error, kind)]
                if matches:
                    state[0] = matches[0] + 1
                else:
                    report(error)
            else:
                report(error)
    return displayed
",
        ),
        (
            "named_handler_nested_elif_call_return_control.py",
            r"async def recover(fetch, report, allow_missing):
    result = None
    try:
        result = await fetch()
    except Exception as error:
        if error.status == 404:
            report(error)
        elif error.status == 403:
            if allow_missing:
                report(error)
            else:
                report(error)
                raise
        else:
            report(error)
            raise error
    return finish(result)
",
        ),
        (
            "named_handler_nested_elif_small_return_target.py",
            r"async def recover(fetch, report, allow_missing):
    result = None
    try:
        result = await fetch()
    except Exception as error:
        if error.status == 404:
            report(error)
        elif error.status == 403:
            if allow_missing:
                report(error)
            else:
                report(error)
                raise
        else:
            report(error)
            raise error
    return result
",
        ),
        (
            "named_handler_nested_try_call_return_control.py",
            r"def recover(fetch, fallback, report, allow_fallback):
    try:
        return fetch()
    except Exception as error:
        if allow_fallback:
            request = error.args
            try:
                result = fallback(request)
                assert result is not None
                return result
            except Exception as nested_error:
                report(nested_error)
        else:
            report(error)
    return finish(None)
",
        ),
        (
            "named_handler_nested_try_small_return_target.py",
            r"def recover(fetch, fallback, report, allow_fallback):
    try:
        return fetch()
    except Exception as error:
        if allow_fallback:
            request = error.args
            try:
                result = fallback(request)
                assert result is not None
                return result
            except Exception as nested_error:
                report(nested_error)
        else:
            report(error)
    return None
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_named_handler_nested_recovery_exit_copies() {
    for (filename, source) in [
        (
            "named_handler_nested_recovery_short_return_target.py",
            r#"def choose(value, first, fallback):
    result = {}
    try:
        result["value"] = primary(value)
    except ValueError as error:
        if first:
            result["value"] = "exists"
        elif fallback:
            try:
                result["value"] = secondary(value)
            except ValueError as recovery:
                if can_recover(recovery):
                    result["value"] = "recovered"
                else:
                    raise
        else:
            raise
    return result
"#,
        ),
        (
            "named_nested_try_normal_exit_copy_order_control.py",
            r"def recover(fetch, retry, report, result, first, fallback, accepted):
    try:
        result[0] = fetch()
    except Exception as error:
        if first(error):
            report(error)
        elif fallback(error):
            try:
                result[0] = retry()
            except Exception as nested_error:
                if accepted(nested_error):
                    result[1] = True
                else:
                    raise
        else:
            raise
    return result
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_interleaved_unnamed_handler_literal_returns() {
    let source = r#"def string_return(primary, secondary, finish):
    try:
        return primary()
    except Exception:
        try:
            try:
                secondary()
            except Exception:
                return ""
            return finish()
        except Exception:
            return "fallback"


def bare_return(primary, secondary, finish):
    try:
        return primary()
    except Exception:
        try:
            try:
                secondary()
            except Exception:
                return
            return finish()
        except Exception:
            return None
"#;
    common::assert_matches_cpython_marshal(
        source,
        "interleaved_unnamed_handler_literal_returns.py",
    );
}

#[test]
fn matches_cpython_shared_handler_nested_join_borrow_return_copies() {
    for alternative in ["elif d", "else"] {
        for returned in ["value", "42"] {
            let source = format!(
                "def f(value, a, b, c, d):\n    try:\n        work()\n    except Error:\n        if a:\n            if b:\n                if c:\n                    value = update(value)\n            else:\n                raise\n        {alternative}:\n            value = update(value)\n    return {returned}\n"
            );
            common::assert_matches_cpython_marshal(&source, "shared_handler_nested_join_return.py");
        }
    }
}
