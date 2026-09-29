use crate::common;

#[test]
fn matches_cpython_async_with_suppression_before_else_and_finally() {
    for body in [
        "async with first:\n    await work()",
        "async with first:\n    async with second:\n        await work()",
    ] {
        for clauses in [
            "        except Error:\n            outcome = None\n            raise\n        else:\n            outcome = True\n        finally:\n            finish(outcome)\n",
            "        except Error:\n            outcome = None\n            raise\n        finally:\n            finish(outcome)\n",
            "        except Error:\n            outcome = None\n            raise\n        else:\n            outcome = True\n",
            "        finally:\n            finish(outcome)\n",
        ] {
            let body = body.replace('\n', "\n            ");
            let source = format!(
                "async def example(manager, first, second):\n    with manager:\n        outcome = False\n        try:\n            {body}\n{clauses}"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "async_with_suppression_before_else_and_finally.py",
            );
        }
    }
}
