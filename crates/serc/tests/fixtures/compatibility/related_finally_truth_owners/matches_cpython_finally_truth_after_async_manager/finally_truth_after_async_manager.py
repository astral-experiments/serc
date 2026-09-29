from __future__ import annotations

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
