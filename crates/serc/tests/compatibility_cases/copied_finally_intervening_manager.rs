use crate::common;

#[test]
fn copied_finally_intervening_manager_target() {
    let source = r"def run(manager, value, release, finish):
    try:
        with manager:
            try:
                return value()
            finally:
                release()
    finally:
        finish()
";
    common::assert_matches_cpython_marshal(source, "intervening_manager_finally_target.py");
}

#[test]
fn copied_finally_intervening_manager_outer_manager_control() {
    let source = r"def run(manager, value, release, finish):
    with manager:
        try:
            try:
                return value()
            finally:
                release()
        finally:
            finish()
";
    common::assert_matches_cpython_marshal(source, "outer_manager_finally_control.py");
}

#[test]
fn copied_finally_intervening_manager_async_target() {
    let source = r"async def run(manager, value, release, finish):
    try:
        async with manager:
            try:
                return await value()
            finally:
                await release()
    finally:
        await finish()
";
    common::assert_matches_cpython_marshal(source, "intervening_async_manager_finally_target.py");
}

#[test]
fn copied_finally_intervening_manager_async_outer_manager_control() {
    let source = r"async def run(manager, value, release, finish):
    async with manager:
        try:
            try:
                return await value()
            finally:
                await release()
        finally:
            await finish()
";
    common::assert_matches_cpython_marshal(source, "outer_async_manager_finally_control.py");
}

#[test]
fn copied_finally_intervening_manager_literal_none_target() {
    let source = r"def run(manager, value, release, finish):
    try:
        with manager:
            try:
                return None
            finally:
                release()
    finally:
        finish()
";
    common::assert_matches_cpython_marshal(source, "intervening_manager_none_finally_target.py");
}

#[test]
fn copied_finally_intervening_manager_literal_none_outer_manager_control() {
    let source = r"def run(manager, value, release, finish):
    with manager:
        try:
            try:
                return None
            finally:
                release()
        finally:
            finish()
";
    common::assert_matches_cpython_marshal(source, "outer_manager_none_finally_control.py");
}
