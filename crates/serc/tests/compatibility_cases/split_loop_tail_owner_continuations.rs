use crate::common;

#[test]
fn retains_branch_owner_before_split_loop_tail() {
    let source = r#"def collect(rows, shallow, limit):
    entries = []
    groups = []
    stopped = False
    try:
        if shallow:
            for row in rows:
                name = row.name
                if not name:
                    continue
                if name.endswith("/"):
                    groups.append({"kind": "group", "name": name})
                else:
                    entry = convert(row)
                    if entry is not None:
                        entries.append(entry)
                if len(entries) + len(groups) >= limit:
                    stopped = True
                    break
        else:
            for row in rows:
                entry = convert(row)
                if entry is None:
                    continue
                entries.append(entry)
                if len(entries) >= limit:
                    stopped = True
                    break
    except Exception:
        recover()
    return entries, groups, stopped
"#;
    common::assert_matches_cpython_marshal(source, "split_loop_tail_branch_before_limit.py");
}

#[test]
fn retains_nested_iterator_break_owner_before_split_condition() {
    let source = r#"def contains_match(records, accept):
    found = False
    failed = False
    for record in records:
        try:
            values = record.values
            if isinstance(values, list):
                for value in values:
                    if accept(value):
                        found = True
                        break
            if not found:
                for key in ("one", "two", "three", "four", "five"):
                    value = getattr(record, key, None)
                    if accept(value):
                        found = True
                        break
            if not found:
                if accept(record.fallback):
                    found = True
        except Exception:
            failed = True
    return found, failed
"#;
    common::assert_matches_cpython_marshal(source, "split_loop_tail_break_before_nested_tail.py");
}

#[test]
fn retains_nested_iterator_break_owner_before_nonterminal_tail() {
    let source = r#"def replace_selected(records, edit):
    for record in records:
        try:
            rows = record.rows
            if isinstance(rows, list):
                selected = None
                for index, row in enumerate(rows):
                    if isinstance(row, dict) and row.get("kind") == "entry" and row.get("key") == "primary":
                        selected = index
                        break
                if selected is None:
                    for index, row in enumerate(rows):
                        if isinstance(row, dict) and row.get("kind") == "entry":
                            selected = index
                            break
                if selected is not None:
                    old = rows[selected].get("value")
                    if isinstance(old, str) and old:
                        rows[selected]["value"] = edit(old)
                    else:
                        rows[selected]["value"] = "default"
                    save(record, rows)
        except Exception:
            recover(record)
    complete()
"#;
    common::assert_matches_cpython_marshal(
        source,
        "split_loop_tail_break_before_nonterminal_tail.py",
    );
}

#[test]
fn matches_cpython_nested_split_loop_tail_shared_jump_locations() {
    for (filename, source) in [
        (
            "split_loop_tail_elif_shared_jump.py",
            r#"def run(items, flags, record):
    for item in items:
        if flags["mode"] == 0:
            record["zero"] += item
        elif flags["mode"] == 1:
            record["prefix"] += item
            if flags["middle"]:
                if flags["inner"]:
                    record["first"] += item
                else:
                    record["second"] += item
                    if flags["left"] and flags["right"]:
                        record["shared"] += item
            else:
                record["fallback"] += item
        else:
            record["other"] += item
        if flags["after"]:
            record["after"] += item
    return record
"#,
        ),
        (
            "split_loop_tail_nested_shared_jump.py",
            r#"def run(items, flags, record):
    for item in items:
        if flags["outer"]:
            if flags["middle"]:
                if flags["inner"]:
                    record["first"] += item
                else:
                    record["second"] += item
                    if flags["left"] and flags["right"]:
                        record["shared"] += item
            else:
                record["fallback"] += item
        else:
            record["other"] += item
        if flags["after"]:
            record["after"] += item
    return record
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn retains_while_break_owner_before_split_source_continuation() {
    for (scope, work) in [
        ("def", "work()"),
        ("def", "yield work()"),
        ("async def", "await work()"),
        ("async def", "yield await work()"),
    ] {
        for condition in ["True", "active"] {
            for conditional_arm in [false, true] {
                let body =
                    format!("while {condition}:\n    if pid == 0:\n        break\n    {work}\n");
                let body = if conditional_arm {
                    format!("if ready:\n{}", common::indent(&body))
                } else {
                    body
                };
                let protected = format!("{body}if next_ready:\n    pass\n");
                let body = format!(
                    "while True:\n    try:\n{}    except Error:\n        pass\n",
                    common::indent(&common::indent(&protected)),
                );
                let source = format!(
                    "{scope} f(ready, next_ready, active, pid):\n{}",
                    common::indent(&body),
                );
                common::assert_matches_cpython_marshal(&source, "while_split_tail_continuation.py");
            }
        }
    }
}
