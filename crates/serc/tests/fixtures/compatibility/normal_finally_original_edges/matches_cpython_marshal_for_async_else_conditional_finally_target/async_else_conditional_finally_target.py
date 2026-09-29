def make_cleanup(waitable, first_gate, report, second_gate):
    async def cleanup():
        status: str = "failed"
        released = False
        try:
            item = await waitable
        except errors.Cancelled:
            status = "cancelled"
            released = True
        except Exception:
            status = "failed"
            released = True
        else:
            result = await terminate(item, message="cleanup failed")
            if not result.confirmed:
                status = "incomplete"
            else:
                status = "finished"
                released = True
            if result.interrupted:
                raise errors.Cancelled
        finally:
            if released and first_gate is not None:
                first_gate.release()
            try:
                if report is not None:
                    report(status, released)
            finally:
                if released and second_gate is not None:
                    await second_gate.release()
    return cleanup
