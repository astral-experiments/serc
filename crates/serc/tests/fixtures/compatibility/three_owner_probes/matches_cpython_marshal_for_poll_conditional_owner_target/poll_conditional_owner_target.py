def run():
    context = Context.create()
    try:
        context.command("setup", "--root", str(context.root), "--count", "1")
        config = codec.decode(
            context.command(
                "--actor",
                "actor-a",
                "acquire",
                "--root",
                str(context.root),
                "--format",
                "data",
                "--id",
                "0",
            ).output
        )
        context.set_stamp(config["name"], BASELINE)
        process = context.spawn(
            "watch",
            config["name"],
            "--root",
            str(context.root),
            "--actor",
            "actor-a",
            "--token",
            config["token"],
            "--duration",
            "1800",
            "--interval",
            "1",
            "--limit",
            "2",
            "--format",
            "data",
        )
        try:
            ready = wait_until(
                lambda: (
                    not bool(context.row(config["name"]).get("expired"))
                    and str(context.row(config["name"]).get("updated") or "") != BASELINE
                ),
                timeout_seconds=5,
            )
            require(ready, "operation did not finish")
            payload = codec.decode(
                context.checked_command(
                    "refresh",
                    "--root",
                    str(context.root),
                    "--format",
                    "data",
                    "--age",
                    "0",
                    "--strict",
                ).output
            )
            require(not bool(payload.get("changed")), "unexpected update")
            output, error = process.communicate(timeout=5)
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
        require(not error.strip(), f"operation reported: {error.strip()}")
        records = parse_lines(output)
        row = context.row(config["name"])
        require(not bool(row.get("expired")), "record expired")
        return {
            "scenario": "poll_cleanup",
            "ok": True,
            "records": len(records),
            "reason": payload["reason"],
            "transitions": ["ready", "updated"],
        }
    finally:
        context.cleanup()
