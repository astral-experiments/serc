use crate::common;

#[test]
fn matches_cpython_finally_owners_after_literal_collection_folding() {
    for (definition, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    if False:\n        yield v0\n"),
        ("async def", "    if False:\n        yield v0\n"),
    ] {
        for collection in [
            "[0.1, 0.5, 0.9]",
            "[0.1, 0.5, 0.9, 1.1]",
            "[0.1, 0.5, 0.9, 1.1, 1.5]",
            "{0.1, 0.5, 0.9}",
            "{0.1, 0.5, 0.9, 1.1}",
            "{0.1, 0.5, 0.9, 1.1, 1.5}",
        ] {
            let source = format!(
                "{definition} f(v0, v1, v2, v3, v4, v5, v6, v7, v8, v9, v10, v11, v12, v13, v14):
{suspension}    try:
        if v0 is None:
            pass
        first, second, third = v1.method(\"x\", {collection}, float(v2))
        print(f\"result={{float(third):.6f}}\")
    finally:
        if v1 is not None:
            pass
"
            );
            common::assert_matches_cpython_marshal(&source, "literal_collection_owner_slots.py");
        }
    }
}

#[test]
fn preserves_finally_owners_for_other_collection_folds() {
    for collection in [
        "[0.1, 0.5, 0.9, 1.1, 1.5, 1.9]",
        "{0.1, 0.5, 0.9, 1.1, 1.5, 1.9}",
        "(0.1, 0.5, 0.9)",
        "[1 + 2, 3, 4]",
        "[-1, 2, 3]",
        "[v0, 2, 3]",
    ] {
        let source = format!(
            "def f(v0, v1, v2, v3, v4, v5, v6, v7, v8, v9, v10, v11, v12, v13):
    try:
        if v0 is None:
            pass
        first, second, third = v1.method(\"x\", {collection}, float(v2))
        print(f\"result={{float(third):.6f}}\")
    finally:
        if v1 is not None:
            pass
"
        );
        common::assert_matches_cpython_marshal(&source, "other_collection_owner_slots.py");
    }
}

#[test]
fn matches_cpython_literal_defaults_before_try_owner() {
    for (filename, source) in [
        (
            "six_literal_defaults_before_try_owner.py",
            r#"async def run(value, invoke):
    if isinstance(value, str):
        value = 0

    async def update(status: str, a=None, b=None, c=None, d=None, e=None, f=None) -> None:
        await invoke(status)

    try:
        if value == 0:
            await update("zero")
    except Exception:
        return
"#,
        ),
        (
            "five_literal_defaults_before_try_owner_boundary.py",
            r#"async def run(value, invoke):
    if isinstance(value, str):
        value = 0

    async def update(status: str, a=None, b=None, c=None, d=None, e=None) -> None:
        await invoke(status)

    try:
        if value == 0:
            await update("zero")
    except Exception:
        return
"#,
        ),
        (
            "nonconstant_defaults_before_try_owner_control.py",
            r#"async def run(value, invoke):
    if isinstance(value, str):
        value = 0

    async def update(status: str, a=value, b=None, c=None, d=None, e=None, f=None) -> None:
        await invoke(status)

    try:
        if value == 0:
            await update("zero")
    except Exception:
        return
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_folded_list_try_else_search_join_locations() {
    for (filename, source) in [
        (
            "try_else_search_small_list_owner_and_location_target.py",
            r#"def search(api):
    found = False
    try:
        while api.pending():
            try:
                result = api.run(["first", "second"], enabled=True)
                values = result.values()
                if not result.ready():
                    pass
                else:
                    for value in values:
                        if api.matches(value):
                            api.accept(value)
                            found = True
                            break
                    if found:
                        break
            except LookupError:
                pass
            api.wait()
    except Exception:
        return False
    return found
"#,
        ),
        (
            "try_else_search_tuple_location_target.py",
            r#"def search(api):
    found = False
    try:
        while api.pending():
            try:
                result = api.run(("first", "second"), enabled=True)
                values = result.values()
                if not result.ready():
                    pass
                else:
                    for value in values:
                        if api.matches(value):
                            api.accept(value)
                            found = True
                            break
                    if found:
                        break
            except LookupError:
                pass
            api.wait()
    except Exception:
        return False
    return found
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
