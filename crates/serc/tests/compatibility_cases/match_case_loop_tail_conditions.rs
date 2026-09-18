use crate::common;

#[test]
fn matches_cpython_nested_match_case_loop_tail_conditions() {
    for kind in [
        "ordinary",
        "generator",
        "async_generator",
        "while_generator",
    ] {
        for condition in ["enabled", "enabled and not ready()", "enabled or ready()"] {
            for has_default in [false, true] {
                let function_prefix = if kind == "async_generator" {
                    "async "
                } else {
                    ""
                };
                let loop_header = match kind {
                    "async_generator" => "async for item in items:",
                    "while_generator" => "while enabled:",
                    _ => "for item in items:",
                };
                let leading = if kind == "ordinary" {
                    "use(item)"
                } else {
                    "yield item"
                };
                let default = if has_default {
                    "            case _:\n                use(item)\n"
                } else {
                    ""
                };
                let source = format!(
                    "{function_prefix}def f(items, item, enabled, ready, check):\n    {loop_header}\n        match item:\n            case First():\n                if check():\n                    {leading}\n                    if {condition}:\n                        action()\n{default}"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "match_case_loop_tail_conditions.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_match_value_case_try_else_loop_tail() {
    common::assert_matches_cpython_marshal(
        r#"def convert(rows, page, decode):
    for fields in rows:
        for field, payload in fields:
            match field:
                case "name":
                    page.name = payload
                case "counters":
                    try:
                        counters = decode(payload)
                    except Exception:
                        pass
                    else:
                        if value := counters.get("created"):
                            page.created = value
                            page.modified = value
                        if value := counters.get("modified"):
                            page.modified = value
                case "date":
                    page.date = payload
                case _:
                    page.other = payload
    return page
"#,
        "match_value_case_try_else_conditional_tail_target.py",
    );
}

#[test]
fn matches_cpython_match_loop_tail_original_backedges() {
    for (filename, source) in [
        (
            "async_match_wildcard_tail_if_backedge_target.py",
            r"async def run(events, emit, enabled):
    async for event in events:
        match event:
            case Event(kind=Kind.START):
                emit(event)
            case _:
                if enabled:
                    emit(event)
",
        ),
        (
            "async_match_wildcard_tail_if_following_call_control.py",
            r"async def run(events, emit, enabled):
    async for event in events:
        match event:
            case Event(kind=Kind.START):
                emit(event)
            case _:
                if enabled:
                    emit(event)
        emit(None)
",
        ),
        (
            "async_match_final_class_value_failure_backedge_target.py",
            r"async def run(events, emit):
    async for event in events:
        match event:
            case Event(kind=Kind.START):
                emit(event)
            case Event(kind=Kind.END):
                emit(event)
",
        ),
        (
            "async_match_final_class_value_following_call_control.py",
            r"async def run(events, emit):
    async for event in events:
        match event:
            case Event(kind=Kind.START):
                emit(event)
            case Event(kind=Kind.END):
                emit(event)
        emit(None)
",
        ),
        (
            "nested_elif_match_wildcard_tail_if_backedge_target.py",
            r"def run(events, emit, enabled, choose):
    for event in events:
        if event is None:
            emit(None)
        elif choose(event):
            match event:
                case Kind.START:
                    emit(event)
                case _:
                    value = choose(event)
                    if value:
                        emit(event)
",
        ),
        (
            "nested_elif_match_wildcard_tail_if_successor_control.py",
            r"def run(events, emit, enabled, choose):
    for event in events:
        if event is None:
            emit(None)
        elif choose(event):
            match event:
                case Kind.START:
                    emit(event)
                case _:
                    value = choose(event)
                    if value:
                        emit(event)
            emit(None)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
