def prepare(client, gate, prompt_for, environment, branch, limit, retries, timeout):
    active_ids: set[str] = set()

    async def process(item: str) -> tuple[str, str, dict | None]:
        async with gate:
            prompt = prompt_for(item)
            metadata = RequestMetadata(environment=environment, branch=branch)
            task_id: str | None = None
            try:
                for attempt in range(retries):
                    try:
                        task_turn = await client.create_task(prompt, metadata=metadata)
                        task_id = task_turn.task.id
                        active_ids.add(task_id)
                        current_active = len(active_ids)
                        if current_active > limit:
                            raise RuntimeError(
                                f'Exceeded active task limit: {current_active} > {limit}'
                            )
                        turn_id = task_turn.turn.id
                        output = await poll_task(client, task_id, turn_id, timeout)
                        return item, task_id, output
                    except Exception as error:
                        if attempt == retries - 1:
                            print(
                                f'Request failed for {item} after {retries} attempts: {error}'
                            )
                            return item, '', None
                        await sleep(2**attempt)
            finally:
                if task_id:
                    active_ids.discard(task_id)
            return item, '', None

    return process
