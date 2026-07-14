use crate::common;

#[test]
fn matches_cpython_marshal_for_coroutine_pre_suspension_multigen_owners() {
    let source = r#"async def direct_any_compare(events, tick):
    assert any(
        resource.get("hello") == "world"
        for event in events
        for resource in event.resources
    )
    await tick()


async def direct_any_boolean(events, tick):
    assert any(
        resource.get("hello") and resource["hello"].startswith("wor")
        for event in events
        for resource in event.resources
    )
    await tick()


async def direct_any_related_compare(events, tick):
    assert any(
        resource.get("hello") == "world"
        for event in events
        for resource in event.related
    )
    await tick()


async def direct_any_related_boolean(events, tick):
    assert any(
        resource.get("hello") and resource["hello"].startswith("world")
        for event in events
        for resource in event.related
    )
    await tick()


async def direct_all(entries, tick):
    assert all(value.ready for entry in entries for value in entry.values)
    await tick()


async def negated_any(entries, tick):
    assert not any(value.ready for entry in entries for value in entry.values)
    await tick()


async def three_clauses(entries, tick):
    assert any(
        value.ready
        for entry in entries
        for part in entry.parts
        for value in part.values
    )
    await tick()


async def filtered_clauses(entries, tick):
    assert any(
        value.ready
        for entry in entries
        if entry.enabled
        for value in entry.values
        if value
    )
    await tick()
"#;
    common::assert_matches_cpython_marshal(source, "coroutine_pre_suspension_multigen_owners.py");
}
