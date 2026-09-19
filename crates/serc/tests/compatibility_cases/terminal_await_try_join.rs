use crate::common;

#[test]
fn terminal_await_try_join_target() {
    let source = r#"async def run(manager, first, second, publish, report, completed):
    async with manager:
        first_saved = False
        second_saved = False
        published = False
        try:
            await first.save()
            first_saved = True
            await second.save()
            second_saved = True
            await publish()
            published = True
        except Exception as error:
            raise RuntimeError("save failed") from error
        finally:
            if not published:
                if first_saved:
                    try:
                        await first.delete()
                    except Exception:
                        report("first rollback failed")
                if second_saved:
                    try:
                        await second.delete()
                    except Exception:
                        report("second rollback failed")
"#;
    common::assert_matches_cpython_marshal(source, "terminal_await_try_join_target.py");
}

#[test]
fn terminal_await_try_join_live_tail_control() {
    let source = r#"async def run(manager, first, second, publish, report, completed):
    async with manager:
        first_saved = False
        second_saved = False
        published = False
        try:
            await first.save()
            first_saved = True
            await second.save()
            second_saved = True
            await publish()
            published = True
        except Exception as error:
            raise RuntimeError("save failed") from error
        finally:
            if not published:
                if first_saved:
                    try:
                        await first.delete()
                    except Exception:
                        report("first rollback failed")
                if second_saved:
                    try:
                        await second.delete()
                    except Exception:
                        report("second rollback failed")
                    completed()
"#;
    common::assert_matches_cpython_marshal(source, "terminal_await_try_join_live_tail_control.py");
}
