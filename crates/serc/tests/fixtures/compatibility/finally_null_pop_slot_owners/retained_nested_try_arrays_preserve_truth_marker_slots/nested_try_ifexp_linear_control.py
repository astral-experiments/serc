def f(resources, run, parse):
    outputs = []
    for resource in resources:
        try:
            async def listing():
                return await resource.list()
            text = run(listing())
            files = []
            try:
                files = parse(text.strip().split("\n")[-1])
            except Exception:
                files = [line for line in text.split("\n") if line]
        except Exception:
            files = []
        outputs.extend(files)
    return outputs
