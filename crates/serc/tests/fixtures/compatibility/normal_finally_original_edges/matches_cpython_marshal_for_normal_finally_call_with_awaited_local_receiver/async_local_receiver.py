async def target(address: str = "https://example.invalid") -> None:
    client = make_client(address=address)
    print("Creating resource...")
    resource = await client.create_resource(kind="local", network=Policy.DEFAULT)
    try:
        print("Preparing the first stage...")
        await resource.worker.execute(["runner", "--script", SETUP_SCRIPT], timeout=6000)
        await resource.worker.execute(["runner", "--script", CONFIG_SCRIPT], timeout=6000)
        print("Running the final stage...")
        result = await resource.worker.execute(["runner", "build", "package"], timeout=180000)
        print(result.decode())
        print("The final stage completed!")
    except Exception as error:
        print("The operation failed:", error)
        try:
            from subprocess import CalledProcessError
        except ImportError:
            CalledProcessError = Exception
        if isinstance(error, CalledProcessError):
            try:
                print("Result:\n", error.result.decode())
            except Exception:
                print("(The result could not be decoded)")
    finally:
        print("Closing resource...")
        await resource.close()
    print("Finished!")
