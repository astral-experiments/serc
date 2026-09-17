use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_finally_truth_after_handler() {
    let source = r"async def stream_events(source, observers, signal, pending, stack):
    try:
        tracker = get_tracker()
        await observers.before()
        async for event in source:
            yield event
        for deferred in pending:
            yield deferred
    except Exception as exc:
        await observers.on_error(exc)
        raise
    finally:
        try:
            if signal and not signal.is_set():
                signal.set()
            if tracker:
                tracker.record_before()
            with scope():
                await observers.done()
            if tracker:
                tracker.record_after()
        finally:
            stack.close()
    audit()
";
    assert_matches_cpython_marshal(source, "finally_truth_after_handler.py");
}

#[test]
fn matches_cpython_finally_truth_after_async_manager() {
    let source = r#"from __future__ import annotations

class TaskSuggestionsTurnCompleterRuntime:
    async def complete(self):
        async with NoOpConversationTreeTransaction(self.initial_tree) as pipeline_convo_txn:
            pipeline = ConversationStreamPipeline()
            await pipeline.construct(convo_txn=pipeline_convo_txn)
            stream_owns_completer = False
            try:
                await pipeline.build_agent()
                async with NoOpConversationTreeTransaction(self.prompt_tree) as prompt_convo_txn:
                    await pipeline.set_conversation(
                        conversation_path=convo_txn_to_path(prompt_convo_txn),
                        body_messages=self.messages or [],
                    )
                    built_streams = await pipeline.build_streams(self.body)
                    stream_owns_completer = True
                    return await self._collect_completion(pipeline, built_streams)
            finally:
                try:
                    if not stream_owns_completer:
                        completer = getattr(pipeline, "completer", None)
                        if completer is not None:
                            await completer.aclose()
                finally:
                    await pipeline.finalize()
"#;
    assert_matches_cpython_marshal(source, "finally_truth_after_async_manager.py");
}
