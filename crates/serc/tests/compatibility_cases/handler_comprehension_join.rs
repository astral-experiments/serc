use crate::common;

#[test]
fn matches_cpython_handler_comprehension_join() {
    common::assert_matches_cpython_marshal(
        r#"def add_comparison_record(store, table_name, expected, selected=None):
    """Record whether selected fields contain the expected value."""
    table = store.get_table(table_name)
    if table is None:
        return f"Table '{table_name}' not found"

    try:
        if expected.replace(".", "", 1).isdigit():
            converted = float(expected)
        else:
            converted = expected

        if selected:
            if selected not in table.data.fields:
                return f"Field {selected} does not exist"
            result = {
                field: table.data[selected].eq(converted).any()
                if field == selected
                else False
                for field in table.data.fields
            }
        else:
            result = {field: table.data[field].eq(converted).any() for field in table.data.fields}

    except ValueError:
        if selected:
            if selected not in table.data.fields:
                return f"Field {selected} does not exist"
            result = {
                field: table.data[selected].eq(expected).any()
                if field == selected
                else False
                for field in table.data.fields
            }
        else:
            result = {field: table.data[field].eq(expected).any() for field in table.data.fields}

    position = table.add_record(result)
    return f"Added record {position + 1} comparing {selected if selected else 'all fields'} with {expected}"
"#,
        "handler_comprehension_join_target.py",
    );
}

#[test]
fn matches_cpython_handler_comprehension_join_fallthrough_control() {
    common::assert_matches_cpython_marshal(
        r#"def add_comparison_record(store, table_name, expected, selected=None):
    """Record whether selected fields contain the expected value."""
    table = store.get_table(table_name)
    if table is None:
        return f"Table '{table_name}' not found"

    try:
        if expected.replace(".", "", 1).isdigit():
            converted = float(expected)
        else:
            converted = expected

        if selected:
            if selected not in table.data.fields:
                return f"Field {selected} does not exist"
            result = {
                field: table.data[selected].eq(converted).any()
                if field == selected
                else False
                for field in table.data.fields
            }
        else:
            result = {field: table.data[field].eq(converted).any() for field in table.data.fields}

    except ValueError:
        if selected:
            if selected not in table.data.fields:
                return f"Field {selected} does not exist"
            result = {
                field: table.data[selected].eq(expected).any()
                if field == selected
                else False
                for field in table.data.fields
            }
        else:
            result = fallback(table.data, expected)

    position = table.add_record(result)
    return f"Added record {position + 1} comparing {selected if selected else 'all fields'} with {expected}"
"#,
        "handler_comprehension_join_control.py",
    );
}
