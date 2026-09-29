class Worker(Base):
    async def run(self, sample):
        result = await super().run(sample)
        accepted = result.items[0].accepted if result.items else None
        output = output = result.metadata.get("response")
        extra = result.extra.copy() if result.extra else {}
        extra["accepted"] = accepted
        extra["output"] = output
        if result.items:
            data = result.metadata.copy()
            data["accepted"] = accepted
            return Result.good(
                worker_name="worker",
                sample_id=sample.sample_id,
                reward_name="worker",
                accepted=True,
                metadata=data,
                extra_metadata=extra,
                metrics=result.metrics,
            )
        else:
            data = result.metadata.copy()
            data["accepted"] = None
            return Result.failed(
                worker_name="worker",
                sample_id=sample.sample_id,
                errors=result.errors,
                answer=result.answer,
                extra_metadata=extra,
                metadata=data,
            )
