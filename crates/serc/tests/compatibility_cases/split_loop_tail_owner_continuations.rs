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
