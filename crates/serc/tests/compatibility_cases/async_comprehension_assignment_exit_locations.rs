use crate::common;

#[test]
fn matches_cpython_async_comprehension_assignment_exit_locations() {
    for expression in [
        "[item async for item in items]",
        "{item async for item in items}",
        "{item: item async for item in items}",
    ] {
        let source = format!(
            "async def f(items):\n    try:\n        values = {expression}\n    except Error:\n        recover()\n    finish(values)\n"
        );
        common::assert_matches_cpython_marshal(
            &source,
            "async_comprehension_assignment_exit_locations.py",
        );
    }
}

#[test]
fn matches_cpython_comprehension_assignment_successor_locations() {
    for source in [
        "async def probe(streams):\n    for stream in streams:\n        result = [item async for item in stream]\n",
        "def probe(stream):\n    result = [\n        item for item in stream\n    ]\n    summary = (\n        \"prefix %s\"\n        % (len(result),)\n    )\n    return summary\n",
    ] {
        common::assert_matches_cpython_marshal(
            source,
            "comprehension_assignment_successor_locations.py",
        );
    }
}
