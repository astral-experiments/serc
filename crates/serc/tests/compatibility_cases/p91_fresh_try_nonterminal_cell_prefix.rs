use crate::common::assert_matches_cpython_marshal;

fn source(captured: &str) -> String {
    format!(
        r"async def run(analyzer, mode, tags, stream, manager, user):
    def captured_values():
        return {captured}

    async with manager:
        try:
            async for item in stream:
                consume(item)
            metrics.increment('stream.success', tags=tags)
        except Exception as error:
            metrics.increment('stream.error', tags=tags)
            report(error)
            raise
        finally:
            analyzer.switch_category(Categories.CLEANUP)
            metrics.increment('stream.total', tags=tags)
            try:
                if mode:
                    metric_name = 'enabled.cleanup'
                else:
                    metric_name = 'disabled.cleanup'
                config = user.get_config(Options.CLEANUP, default={{'enabled': False}})
                if config.get('enabled', False):
                    pending.append(schedule(monitor(user=user, metric_name=metric_name)))
            except Exception:
                logger.exception('cleanup failed')
        await completed()
    return mode
"
    )
}

#[test]
fn matches_cpython_fresh_try_nonterminal_cell_prefix() {
    assert_matches_cpython_marshal(
        &source("analyzer, mode, tags"),
        "fresh_try_nonterminal_cell_prefix.py",
    );
}

#[test]
fn matches_cpython_fresh_try_nonterminal_local_prefix_control() {
    assert_matches_cpython_marshal(
        &source("mode"),
        "fresh_try_nonterminal_local_prefix_control.py",
    );
}
