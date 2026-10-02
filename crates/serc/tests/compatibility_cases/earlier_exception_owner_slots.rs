use crate::common;

#[test]
fn matches_cpython_finally_owner_after_negative_literal_folding() {
    for literal in ["-1", "-257", "-1.5", "-2j"] {
        for calls in [1, 2, 3, 6] {
            for condition in ["a", "a is not None", "a >= 0"] {
                let body = "        use(a, b, c, d)\n".repeat(calls);
                let source = format!(
                    "def f(a, b, c, d):\n    try:\n{body}        value = {literal}\n    finally:\n        if {condition}:\n            cleanup()\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "negative_literal_finally_owner.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_negative_literal_owner_across_scope_and_branch_boundaries() {
    for (definition, tail) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield a\n"),
        ("async def", "    yield a\n"),
    ] {
        for branch in ["", "        if flag:\n            value = -257\n"] {
            let source = format!(
                "{definition} f(a, b, c, d, flag):\n    try:\n        use(a, b, c, d)\n        use(a, b, c, d)\n{branch}        value = -1\n    finally:\n        if a is not None:\n            cleanup()\n{tail}"
            );
            common::assert_matches_cpython_marshal(&source, "negative_literal_owner_boundary.py");
        }
    }
}

#[test]
fn matches_cpython_normal_finally_reuses_earlier_try_owner() {
    for (definition, tail) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield value\n"),
        ("async def", "    yield value\n"),
    ] {
        for calls in [1, 2, 3, 4, 8] {
            for condition in ["value is not None", "value"] {
                let body = "        work()\n".repeat(calls);
                let source = format!(
                    "{definition} f(value, obj, flag, limit, manager):\n    try:\n{body}    finally:\n        if {condition}:\n            after()\n{tail}"
                );
                common::assert_matches_cpython_marshal(&source, "earlier_try_owner_slots.py");
            }
        }
    }
}

#[test]
fn matches_cpython_finally_owner_after_source_branch_join() {
    for (definition, tail) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield value\n"),
        ("async def", "    yield value\n"),
    ] {
        let source = format!(
            "{definition} f(value, obj, flag, limit, manager):\n    try:\n        if flag:\n            before()\n        work()\n        work()\n        work()\n    finally:\n        if value is not None:\n            after()\n{tail}"
        );
        common::assert_matches_cpython_marshal(&source, "source_join_finally_owner.py");
    }
}

#[test]
fn matches_cpython_finally_owner_slot_before_protected_prefix() {
    for condition in ["value is not None", "value"] {
        let prefix = "    before()\n".repeat(100);
        let source = format!(
            "def f(value):\n{prefix}    try:\n        work()\n        work()\n        work()\n    finally:\n        if {condition}:\n            after()\n"
        );
        common::assert_matches_cpython_marshal(&source, "before_protected_prefix_owner.py");
    }
}

#[test]
fn matches_cpython_targeted_finally_entry_rejects_prior_array_history() {
    for definition in ["def", "async def"] {
        for body in [
            "        if flag:\n            work()\n            work()\n            work()\n",
            "        if flag:\n            work()\n            work()\n            work()\n        else:\n            other()\n            other()\n            other()\n",
        ] {
            for condition in ["value is not None", "value"] {
                let source = format!(
                    "{definition} f(value, flag):\n    try:\n{body}    finally:\n        if {condition}:\n            after()\n"
                );
                common::assert_matches_cpython_marshal(&source, "targeted_finally_entry_owner.py");
            }
        }
    }
}

#[test]
fn normal_finally_none_marker_reuses_removed_pop_slot() {
    let source =
        normal_finally_pop_slot_source(r#"create_client(f"{ROOT}/{key}/resource", credential)"#);
    common::assert_matches_cpython_marshal(&source, "normal_finally_removed_pop_slot.py");
}

#[test]
fn normal_finally_none_marker_keeps_slot_after_removed_pop() {
    let source =
        normal_finally_pop_slot_source(r#"client_factory(f"{ROOT}/{key}/resource", credential)"#);
    common::assert_matches_cpython_marshal(&source, "normal_finally_slot_after_removed_pop.py");
}

#[test]
fn normal_finally_none_marker_keeps_slot_before_removed_pop() {
    let source = normal_finally_pop_slot_source(
        r#"normalize(create_client(f"{ROOT}/{key}/resource", credential))"#,
    );
    common::assert_matches_cpython_marshal(&source, "normal_finally_slot_before_removed_pop.py");
}

fn normal_finally_pop_slot_source(client_call: &str) -> String {
    format!(
        r#"from contextlib import contextmanager

@contextmanager
def collect(key: str, client_factory) -> Iterator[None]:
    with GATE:
        local = LOCKS.setdefault(key, Lock())

    with local:
        if not ROOT.startswith("remote://"):
            yield
            return

        from example import Credential

        credential = Credential()
        client = {client_call}
        lease = None
        try:
            lease = acquire(client)
            yield
        finally:
            if lease is not None:
                lease.release()
            client.close()
            credential.close()
"#
    )
}

#[test]
fn copied_return_none_marker_reuses_retained_pop_slot() {
    let source =
        copied_return_pop_slot_source(r#"request_json(http, "GET", f"/sessions/{session_id}")"#);
    common::assert_matches_cpython_marshal(&source, "copied_return_retained_pop_slot.py");
}

#[test]
fn copied_return_none_marker_preserves_slot_after_pop() {
    let source =
        copied_return_pop_slot_source(r#"request(http, "GET", f"/sessions/{session_id}")"#);
    common::assert_matches_cpython_marshal(&source, "copied_return_slot_after_pop.py");
}

#[test]
fn copied_return_none_marker_preserves_authored_return_slot() {
    let source = copied_return_pop_slot_source(
        r#"normalize(request_json(http, "GET", f"/sessions/{session_id}"))"#,
    );
    common::assert_matches_cpython_marshal(&source, "copied_return_authored_return_slot.py");
}

fn copied_return_pop_slot_source(request: &str) -> String {
    format!(
        r#"async def main(http, request, stream_output):
    session_id = None
    sandbox = None
    try:
        session_id = "session"
        sandbox = http.sandbox()
        await stream_output(http, session_id, sandbox)
        final_session = {request}
        print(f"final status: {{final_session['status']}}")
        return 0
    finally:
        if session_id is not None:
            with suppress(Exception):
                request_json(http, "POST", f"/sessions/{{session_id}}/stop", {{}})
        if sandbox is not None:
            await sandbox.close()
"#
    )
}

#[test]
fn copied_call_return_none_marker_without_handler_control() {
    let source = copied_call_return_none_source().replace(
        "    except Exception:\n        return Result(value=None)\n",
        "",
    );
    common::assert_matches_cpython_marshal(&source, "copied_call_return_none_without_handler.py");
}

#[test]
fn cleanup_truth_marker_preserves_owner_after_guard_target_copy() {
    for managers in ["trace('operation')", "trace('operation'), counter()"] {
        for condition in ["tracker", "tracker and not self.disabled"] {
            for work in ["prepare()", "self.prepare()"] {
                let source = format!(
                    "async def f(self, cached, enabled, tracker, item, tag):\n    with {managers}:\n        if cached is not None:\n            if enabled:\n                try:\n                    {work}\n                finally:\n                    if {condition}:\n                        tracker.record('done')\n            record(self.renderer, 'event', outcome='success', extra_tags=['hit', tag, 'source'], count=item.count())\n        elif self.conversation:\n            pass\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "cleanup_truth_guard_target_copy.py",
                );
            }
        }
    }
}

fn copied_call_return_none_source() -> &'static str {
    include_str!(
        "../fixtures/compatibility/earlier_exception_owner_slots/copied_call_return_none_marker_after_completed_async_manager/copied_call_return_none_after_async_manager.py"
    )
}

fn missing_not_taken_owner_duplicate_store_source() -> &'static str {
    include_str!(
        "../fixtures/compatibility/earlier_exception_owner_slots/missing_not_taken_owner_after_duplicate_store/missing_not_taken_owner_after_duplicate_store.py"
    )
}

#[test]
fn missing_not_taken_owner_single_store_control() {
    let source = missing_not_taken_owner_duplicate_store_source().replace(
        "output = output = result.metadata.get(\"response\")",
        "output = result.metadata.get(\"response\")",
    );
    common::assert_matches_cpython_marshal(
        &source,
        "missing_not_taken_owner_single_store_control.py",
    );
}

fn cell_try_else_ternary_owner_source() -> &'static str {
    include_str!(
        "../fixtures/compatibility/earlier_exception_owner_slots/cell_try_else_ternary_owner/cell_try_else_ternary_owner.py"
    )
}

#[test]
fn cell_try_else_ternary_owner_uncaptured_control() {
    let source = cell_try_else_ternary_owner_source().replace(
        "float(item[\"value\"]) <= threshold",
        "float(item[\"value\"]) <= 0",
    );
    common::assert_matches_cpython_marshal(
        &source,
        "cell_try_else_ternary_owner_uncaptured_control.py",
    );
}

#[test]
fn copied_return_none_marker_reuses_owned_suffix_after_try_else() {
    for call in [
        "record(action)",
        "logger().info('message', extra=details | {'key': str(key)})",
        "logger().info('message', extra=details | {'key': normalize(str(key))})",
    ] {
        for condition in ["action is not None", "action is None"] {
            let source = format!(
                "async def f(value, enabled, action, details, key):\n    try:\n        if value is None:\n            if enabled:\n                try:\n                    prepare()\n                except Error:\n                    pass\n                else:\n                    {call}\n                    return action\n            else:\n                pass\n    finally:\n        if {condition}:\n            finish()\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "copied_return_none_try_else_suffix.py",
            );
        }
    }
}

#[test]
fn copied_value_return_truth_marker_reuses_paired_pop_cells() {
    for value in ["result, output", "output, result"] {
        for condition in ["cleanup", "cleanup and resource is not None"] {
            let source = format!(
                "async def f(a, b, c, d, e, g, h, i, j, resource, k, l, enabled, m, n, o, cleanup, output, r, s, t, u):\n    try:\n        try:\n            pass\n        except Error as error:\n            if enabled:\n                return None\n        result = normalize(value.extract())\n        record(key, f'finished {{label}} in {{clock.now() - start:.2f}} seconds')\n        return {value}\n    except Cancelled:\n        pass\n    except Error as error:\n        pass\n    finally:\n        if {condition}:\n            pass\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "copied_value_return_truth_pop_cells.py",
            );
        }
    }
}
